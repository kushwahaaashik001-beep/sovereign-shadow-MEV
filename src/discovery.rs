use ethers::prelude::*;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use crate::factory_scanner::{NewPoolEvent, V2PoolData};
use crate::models::DexName;
use crate::rpc_manager::RpcManager;
use tracing::{info, warn, debug};
use tokio::sync::broadcast;
use rustc_hash::FxHashMap;
use anyhow::Result;

// Factory ABI for fast discovery
abigen!(
    IFactory,
    r#"[
        function allPairsLength() external view returns (uint256)
        function allPairs(uint256) external view returns (address)
        function allPoolsLength() external view returns (uint256)
        function allPools(uint256) external view returns (address)
    ]"#
);

// Pool ABI to fetch tokens during bootstrap
abigen!(
    IPool,
    r#"[
        function token0() external view returns (address)
        function token1() external view returns (address)
    ]"#
);

/// God-level sync function to bootstrap pools from factories
pub async fn sync_initial_pools(
    rpc_manager: Arc<RpcManager>,
    pool_tx: broadcast::Sender<NewPoolEvent>,
    v2_factory: Address,
    aero_factory: Address,
    limit: usize,
) -> Result<()> {
    if v2_factory.is_zero() && aero_factory.is_zero() { return Ok(()); }
    info!("🌊 [ZENITH] Starting Pool Bootstrap using {} RPC keys. (Limit: {})", rpc_manager.provider_count(), limit);
    let shared_count = Arc::new(AtomicUsize::new(0));

    // 1. Sync Uniswap V2 Pairs
    if let Err(e) = fetch_and_dispatch(rpc_manager.clone(), &pool_tx, v2_factory, "allPairs", limit, shared_count.clone()).await {
        warn!("⚠️ V2 Factory sync failed: {}. Check factory address and RPC health.", e);
    }
    // 2. Sync Aerodrome Pools
    if !aero_factory.is_zero() {
        if let Err(e) = fetch_and_dispatch(rpc_manager.clone(), &pool_tx, aero_factory, "allPools", limit, shared_count.clone()).await {
            warn!("⚠️ Aerodrome Factory sync failed: {}", e);
        }
    }

    info!("🚀 Bootstrap complete! Check detector logs for Pool count.");
    Ok(())
}

async fn fetch_and_dispatch(
    rpc_manager: Arc<RpcManager>,
    tx: &broadcast::Sender<NewPoolEvent>,
    factory_addr: Address,
    method: &str,
    limit: usize,
    count: Arc<AtomicUsize>,
) -> Result<()> {
    let factory = IFactory::new(factory_addr, rpc_manager.get_next_provider());

    let total_len = if method == "allPairs" {
        match factory.all_pairs_length().call().await {
            Ok(len) => len,
            Err(e) => {
                warn!("⚠️ Factory {:?} does not support 'allPairsLength'. Skipping V2 sync. Error: {}", factory_addr, e);
                return Ok(());
            }
        }
    } else {
        match factory.all_pools_length().call().await {
            Ok(len) => len,
            Err(e) => {
                warn!("⚠️ Factory {:?} does not support 'allPoolsLength'. Skipping Aerodrome sync. Error: {}", factory_addr, e);
                return Ok(());
            }
        }
    };

    let total = total_len.as_u32() as usize;
    let actual_fetch_count = std::cmp::min(total, limit);
    // Pillar Z: Zenith Strategy - Fetch LATEST pools first (most liquid/active)
    let dex_type = if method == "allPairs" { "UniswapV2/Standard" } else { "Aerodrome/Solidly" };
    let start_idx = if total > actual_fetch_count { total - actual_fetch_count } else { 0 };
    info!("🔍 [{}] Factory {:?} has {} pools. Loading LATEST {} pools...", dex_type, factory_addr, total, actual_fetch_count);

    // Pillar Z: Multicall Batching for Token Discovery (CU Shield)
    let multicall_address: Address = "0xcA11bde05977b3631167028862bE2a173976CA11".parse().unwrap();
    let mut pool_addresses = Vec::new(); // Keep this

    // Optimized: Fetch pool addresses in batches to prevent rate limits
    let mut indices = Vec::with_capacity(actual_fetch_count);
    for i in start_idx..total { indices.push(U256::from(i)); }

    for chunk in indices.chunks(100) { // Aggressive batching for server deployment
        if count.load(Ordering::Relaxed) >= limit { break; }

        let mut call_data_list = Vec::new();
        for &idx in chunk {
            let data = if method == "allPairs" {
                factory.all_pairs(idx).calldata().unwrap()
            } else {
                factory.all_pools(idx).calldata().unwrap()
            };
            call_data_list.push(Call3 {
                target: factory_addr,
                allow_failure: true,
                call_data: data,
            });
        }

        let multicall_for_addrs = IMulticall3::new(multicall_address, rpc_manager.get_next_provider());
        if let Ok(results) = multicall_for_addrs.aggregate_3(call_data_list).call().await {
            for res in results {
                if res.0 && res.1.len() >= 32 {
                    let pool_addr = Address::from_slice(&res.1[12..32]);
                    if !pool_addr.is_zero() && pool_addr != factory_addr {
                        pool_addresses.push(pool_addr);
                    }
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await; // Safer throttle for 3500+ pools
    }

    // Multicall ABI for aggregate3
    abigen!(
        IMulticall3,
        r#"[
            struct Call3 { address target; bool allowFailure; bytes callData; }
            struct Result3 { bool success; bytes returnData; }
            function aggregate3(Call3[] calldata calls) external view returns (Result3[] memory returnData)
        ]"#
    );

    let multicall = IMulticall3::new(multicall_address, rpc_manager.get_next_provider());
    let mut calls = Vec::new();
    let mut pool_addr_map = FxHashMap::default(); // To map call index back to pool address

    // Prepare calls for multicall
    for (_idx, &pool_addr) in pool_addresses.iter().enumerate() {
        let pool_contract = IPool::new(pool_addr, rpc_manager.get_next_provider());
        if let Some(t0_calldata) = pool_contract.token_0().calldata() {
            calls.push((pool_addr, true, t0_calldata));
            pool_addr_map.insert(calls.len() - 1, (pool_addr, true)); // true for token0
        }
        if let Some(t1_calldata) = pool_contract.token_1().calldata() {
            calls.push((pool_addr, true, t1_calldata));
            pool_addr_map.insert(calls.len() - 1, (pool_addr, false)); // false for token1
        }
    }

    // Process in chunks to avoid RPC size limits
    for (chunk_idx, chunk_calls) in calls.chunks(crate::state_mirror::MULTICALL_BATCH_SIZE).enumerate() { 
        if count.load(Ordering::Relaxed) >= limit { break; }

        let chunk_offset = chunk_idx * crate::state_mirror::MULTICALL_BATCH_SIZE;
        let mut multicall_requests = Vec::new();
        for (target, allow_failure, calldata) in chunk_calls {
            multicall_requests.push(Call3 {
                target: *target,
                allow_failure: *allow_failure,
                call_data: calldata.clone(),
            });
        }

        if multicall_requests.is_empty() { continue; }

        // Pillar Z: Exponential Backoff for Bootstrap
        let mut success = false;
        let mut retries = 0;
        while !success && retries < 3 {
            match multicall.aggregate_3(multicall_requests.clone()).call().await {
            Ok(results) => {
                let mut pool_data: FxHashMap<Address, (Address, Address)> = FxHashMap::default();
                for (i, result) in results.into_iter().enumerate() {
                    if result.0 {
                        if let Some(&(pool_addr, is_token0)) = pool_addr_map.get(&(chunk_offset + i)) {
                            if result.1.len() >= 32 {
                                let token_addr = Address::from_slice(&result.1[12..32]);
                                let entry = pool_data.entry(pool_addr).or_default();
                                if is_token0 { entry.0 = token_addr; } else { entry.1 = token_addr; }
                            }
                        }
                    }
                }

                for (pool_addr, (token_0, token_1)) in pool_data {
                    if count.load(Ordering::Relaxed) >= limit { break; }

                    // Pillar Z: Core Token Filter - Only accept pools with high-liquidity assets
                    let has_core = crate::constants::CORE_TOKENS.contains(&token_0) || crate::constants::CORE_TOKENS.contains(&token_1);

                    if !token_0.is_zero() && !token_1.is_zero() && has_core {
                        let dex_name = if method == "allPairs" { DexName::UniswapV2 } else { DexName::Aerodrome };
                        let _ = tx.send(NewPoolEvent::V2(V2PoolData { pair: pool_addr, token_0, token_1, dex_name }));
                        count.fetch_add(1, Ordering::Relaxed);
                    } else {
                        debug!("🚫 [FILTER] Skipping pool {} - No CORE tokens (WETH/USDC/etc) found.", pool_addr);
                    }
                }
                success = true;
            }
            Err(e) => {
                warn!("⚠️ Multicall for factory {:?} failed: {}", factory_addr, e);
                retries += 1;
                tokio::time::sleep(tokio::time::Duration::from_secs(1 * retries)).await;
            }
        }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await; // Fast rotation
    }

    Ok(())
}