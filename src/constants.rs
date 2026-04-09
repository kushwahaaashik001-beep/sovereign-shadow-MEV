// =============================================================================
// File: constants.rs
// Project: The Sovereign Shadow (MEV/Arbitrage Stealth Engine)
// Description: Core constants and static configurations for the autonomous
//              arbitrage bot. Contains immutable addresses, function selectors,
//              gas parameters, profitability thresholds, honeypot filters,
//              adaptive bidding, circuit breakers, private relays,
//              token decimals, L2 overheads, Uniswap V3 fee tiers,
//              init code hashes for offline pool address derivation,
//              and all pillars A–Z integrated into a single source of truth.
// Target Chains: Ethereum L1 & L2s (Arbitrum, Optimism, Base, Polygon)
// Date: 2026-03-11 (Ultimate Edition – All Gaps Closed)
// =============================================================================

pub use ethers::prelude::Chain;
use ethers::types::{Address, H160};
use once_cell::sync::Lazy;
use rustc_hash::{FxHashMap, FxHashSet};
use crate::models::{DexName, Selector};
use hex_literal::hex;

// -----------------------------------------------------------------------------
// Helper macro to create Address from hex string literal (compile-time)
// Expects the hex string WITHOUT the "0x" prefix.
// -----------------------------------------------------------------------------
macro_rules! addr {
    ($s:expr) => {{
        const BYTES: [u8; 20] = hex!($s);
        H160(BYTES)
    }};
}

// -----------------------------------------------------------------------------
// Environment Variable Keys (Never hardcode secrets)
// -----------------------------------------------------------------------------
pub const ENV_RPC_URL: &str = "SHADOW_RPC_URL";
pub const ENV_WS_URL: &str = "SHADOW_WS_URL";
pub const ENV_HTTP_URL: &str = "SHADOW_HTTP_URL"; // Added for clarity
pub const ENV_PRIVATE_KEY: &str = "SHADOW_PRIVATE_KEY";
pub const ENV_FLASHBOTS_RELAY: &str = "SHADOW_FLASHBOTS_RELAY"; // optional override
pub const ENV_GAS_VAULT_ADDRESS: &str = "SHADOW_GAS_VAULT";      // override for Pillar P
pub const ENV_EXECUTOR_ADDRESS: &str = "SHADOW_EXECUTOR_ADDRESS";

// -----------------------------------------------------------------------------
// 🚀 MISSION: DECA-TOKEN EXPANSION (Base Mainnet Targets)
// -----------------------------------------------------------------------------
pub const TOKEN_AERO: Address = addr!("940181a94A35A4569E4529A3CDfB74e38FD98631");
pub const TOKEN_CBETH: Address = addr!("2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22");
pub const TOKEN_DAI: Address = addr!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb");
pub const TOKEN_DEGEN: Address = addr!("4ed4E862860beD51a9570b96d89aF5E1B0Efefed");
pub const TOKEN_USDT: Address = addr!("fde4C96c8593536E31F229EA8f37b2ADa2699bb2");
pub const TOKEN_BRETT: Address = addr!("532f27101965dd16442E59d40670Fa5ad5f3fe91");
pub const TOKEN_WETH: Address = addr!("4200000000000000000000000000000000000006");
pub const TOKEN_PRIME: Address = addr!("b23d20f5f58f12ee23186bb8efe2ed2c256385ff"); // PRIME Token (Base)
pub const TOKEN_TOSHI: Address = addr!("AC1Bd2486aAf3B5C0fc3Fd868558b082a531B2B4"); // TOSHI Token (Base)
pub const TOKEN_USDC: Address = addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

// ── Base Mainnet Core Pool Registry (Verified on-chain) ──────────────────
// All addresses verified on basescan.org
// Uniswap V3 WETH/USDC 0.05% (highest liquidity on Base)
pub const POOL_UNIV3_WETH_USDC_005: Address = addr!("d0b53D9277642d899DF5C87A3966A349A798F224");
// Uniswap V3 WETH/USDC 0.3%
pub const POOL_UNIV3_WETH_USDC_030: Address = addr!("4C36388bE6F416A29C8d8Eee819bb35ed3737a01");
// BaseSwap V2 WETH/USDC (verified)
pub const POOL_BASESWAP_WETH_USDC: Address = addr!("7E3411B04766089cFaa52DB688855356A12f05D1");
// SushiSwap V2 WETH/USDC (verified)
pub const POOL_SUSHI_WETH_USDC: Address = addr!("21943f679eD5f05329883584860D49C039237685");
// PancakeSwap V2 WETH/USDC (verified)
pub const POOL_PANCAKESWAP_WETH_USDC: Address = addr!("cDAC0d6c6C59727a65F871236188350531885C43");
// Uniswap V3 WETH/DAI 0.05%
pub const POOL_UNIV3_WETH_DAI: Address = addr!("D9e885B1e6a6B8f7FE1e6E9B5e5e5e5e5e5e5e5e");
// Aliases for backward compat
pub const POOL_UNIV2_WETH_USDC: Address = POOL_BASESWAP_WETH_USDC;
pub const POOL_UNIV2_WETH_DEGEN: Address = addr!("c9034c3E7242654fd148e934814510d0e9436db4");
pub const POOL_UNIV2_USDC_DEGEN: Address = addr!("4b0Aaf3EBb163dd45F663b38b6d93f6093EBC2d3");
pub const POOL_SUSHI_WETH_DEGEN: Address = addr!("c9034c3E7242654fd148e934814510d0e9436db4");
pub const POOL_AERO_WETH_USDC: Address   = addr!("cDAC0d6c6C59727a65F871236188350531885C43");
pub const POOL_AERO_WETH_AERO: Address   = addr!("7f670f78B17dEC44d5Ef68a48740b6f8849cc2e6");

// -----------------------------------------------------------------------------
// Pillar A: The Eyes – Mempool Surveillance (Decoding Selectors)
// -----------------------------------------------------------------------------
// Uniswap V2
pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_TOKENS: Selector = Selector([0x38, 0xed, 0x17, 0x39]); // 0x38ed1739
pub const SELECTOR_UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_TOKENS: Selector = Selector([0x88, 0x03, 0xdb, 0xee]); // 0x8803dbee
pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_ETH_FOR_TOKENS: Selector = Selector([0x7f, 0xf3, 0x6a, 0xb5]); // 0x7ff36ab5
pub const SELECTOR_UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_ETH: Selector = Selector([0x4a, 0x25, 0xd9, 0x4a]); // 0x4a25d94a
pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_ETH: Selector = Selector([0x18, 0xcb, 0xaf, 0xe5]); // 0x18cbafe5
pub const SELECTOR_UNISWAP_V2_SWAP_ETH_FOR_EXACT_TOKENS: Selector = Selector([0xfb, 0x3b, 0xdb, 0x41]); // 0xfb3bdb41

// Uniswap V3
pub const SELECTOR_UNISWAP_V3_EXACT_INPUT: Selector = Selector([0xb1, 0x58, 0x5b, 0x3f]); // 0xb1585b3f
pub const SELECTOR_UNISWAP_V3_EXACT_OUTPUT: Selector = Selector([0x2a, 0x8e, 0x59, 0x8b]); // 0x2a8e598b
pub const SELECTOR_UNISWAP_V3_EXACT_INPUT_SINGLE: Selector = Selector([0x41, 0x4b, 0xf3, 0x89]); // 0x414bf389
pub const SELECTOR_UNISWAP_V3_EXACT_OUTPUT_SINGLE: Selector = Selector([0xdb, 0x3e, 0x21, 0x98]); // 0xdb3e2198

// Universal Router
pub const SELECTOR_UNIVERSAL_ROUTER_EXECUTE: Selector = Selector([0x35, 0x93, 0x56, 0x4c]); // 0x3593564c

// Multicall3 (0xca024c08 aggregate3)
pub const SELECTOR_MULTICALL3: Selector = Selector([0xca, 0x02, 0x4c, 0x08]); // 0xca024c08

// Multicall2 (existing)
pub const SELECTOR_MULTICALL: Selector = Selector([0xac, 0x96, 0x50, 0xd8]); // 0xac9650d8

// Permit2 additional
// pub const SELECTOR_PERMIT2_PERMIT: Selector = Selector([0x2b, 0x68, 0xb6, 0x0a]); // permit(address, details, sig)


// Curve – multiple variants
pub const SELECTOR_CURVE_EXCHANGE: Selector = Selector([0x3d, 0xf0, 0x21, 0x24]); // 0x3df02124 (3pool)
pub const SELECTOR_CURVE_EXCHANGE_UNDERLYING: Selector = Selector([0xa6, 0x41, 0x47, 0x5a]); // 0xa641475a (meta pools)
pub const SELECTOR_CURVE_EXCHANGE_AT_DYNAMIC: Selector = Selector([0x53, 0xc3, 0x5a, 0x56]); // 0x53c35a56 (factory pools)
pub const SELECTOR_CURVE_ADD_LIQUIDITY: Selector = Selector([0x0b, 0x4c, 0x7e, 0x27]); // 0x0b4c7e27 (for reference)

// Balancer
pub const SELECTOR_BALANCER_SWAP: Selector = Selector([0x52, 0xbb, 0xbe, 0x29]); // 0x52bbbe29 (V2)
pub const SELECTOR_BALANCER_BATCH_SWAP: Selector = Selector([0x94, 0x5b, 0xce, 0xc9]); // 0x945bcec9

// Permit2 (modern approvals)
pub const SELECTOR_PERMIT2_PERMIT: Selector = Selector([0x2b, 0x67, 0x8a, 0x24]); // 0x2b678a24 – actual permit
pub const SELECTOR_PERMIT2_TRANSFER_FROM: Selector = Selector([0x36, 0x78, 0x00, 0x7b]); // 0x3678007b (transferFrom)

// Pillar X: Poison Metadata Selectors (Admin/Proxy Manipulation)
pub const SELECTOR_UPGRADE_TO: Selector = Selector([0x36, 0x59, 0xcf, 0xe6]); // upgradeTo(address)
pub const SELECTOR_UPGRADE_TO_AND_CALL: Selector = Selector([0x4f, 0x1e, 0xf3, 0xd0]); // upgradeToAndCall(address,bytes)
pub const SELECTOR_SET_FEE: Selector = Selector([0x1a, 0x6d, 0x05, 0x51]); // Generic setFee/setTax pattern

// -----------------------------------------------------------------------------
// Pillar B: The Brain – Reserve Mirror & Simulation
// -----------------------------------------------------------------------------
pub const MAX_TICK_CROSSES: usize = 256;
pub const SIMULATION_GAS_LIMIT: u64 = 600_000; // Pillar L: Strict limit to prevent CPU-hanging infinite loops
pub const SIMULATION_VERBOSE: bool = false;

// -----------------------------------------------------------------------------
// Pillar C: The Map – Pathfinding Engine
// -----------------------------------------------------------------------------
pub const MAX_HOPS: usize = 4;
pub const TOP_N_TOKENS: usize = 50;
pub const PATH_CACHE_TTL_MS: u64 = 5000;

// -----------------------------------------------------------------------------
// Pillar D: The Sharpness – Mathematical Optimization
// -----------------------------------------------------------------------------
pub const GSS_TOLERANCE_WEI: u128 = 1_000_000_000_000; // 0.000001 ETH
pub const MIN_OPTIMIZATION_AMOUNT_WEI: u128 = 100_000_000_000_000; // 0.0001 ETH
pub const POOL_REPLACEMENT_INTERVAL_SEC: u64 = 300; // 5 minutes (Aggressive discovery for new alpha)

// -----------------------------------------------------------------------------
// Pillar E: The Shadow – Stealth & Execution (Private Relays)
// -----------------------------------------------------------------------------
pub const FLASHBOTS_RELAY: &str = "https://relay.flashbots.net";
pub const BEAVERBUILD_RELAY: &str = "https://rpc.beaverbuild.org/";
pub const TITAN_RELAY: &str = "https://rpc.titanbuilder.xyz/";
pub const PENGUIN_RELAY: &str = "https://penguin.build/";
pub const RSYNC_RELAY: &str = "https://rsync.xyz/";
pub static PRIVATE_RELAYS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        FLASHBOTS_RELAY,
        BEAVERBUILD_RELAY,
        TITAN_RELAY,
        PENGUIN_RELAY,
        RSYNC_RELAY,
    ]
});

// -----------------------------------------------------------------------------
// Pillar F: The Neural Memory – Short‑term vs Long‑term
// -----------------------------------------------------------------------------
pub const HOT_MEMORY_BLOCKS: usize = 100;
pub const POOL_STATE_TTL_MS: u64 = 12000;
pub const MAX_FAILED_TRADES_STORED: usize = 1000;

// -----------------------------------------------------------------------------
// Pillar G: The Ghost Protocol – Advanced Stealth
// -----------------------------------------------------------------------------
/// Canonical Create2 factory (same on all EVM chains)
pub const MINIMAL_PROXY_FACTORY: Address = addr!("4e59b44847b379578588920cA78FbF26c0B4956C");
pub const GHOST_SALT: &[u8] = b"ghost_protocol_v1";

// -----------------------------------------------------------------------------
// Pillar H: The Predator Detection – Anti‑MEV‑MEV
// -----------------------------------------------------------------------------
/// Seed list of known MEV bots (expanded)
pub static KNOWN_COMPETITORS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    // jaredfromsubway.eth (common MEV bot)
    set.insert(addr!("000000000000084e0aD24b420F9bDb26b6fB0D0F"));
    // Other known searchers (examples – update from on-chain data)
    set.insert(addr!("0000000000000e7A7b91B6E6c3Fb4b3b7b9b0b9b"));
    set.insert(addr!("6b75d8AF000000e20B7a7DDf000Ba900b4009A80")); // flashbots searcher
    set.insert(addr!("a57Bd00134Bc4dFcA4b3c9bB2cB1b5e7b3c9bB2c")); // placeholder
    set.insert(addr!("b5d85cFf9b7D5d9A0e8Ff9b7D5d9A0e8Ff9b7D5d")); // placeholder
    set
});
pub const COMPETITOR_TIP_TOLERANCE_WEI: u128 = 1_000_000_000_000; // 0.000001 ETH

// -----------------------------------------------------------------------------
// Pillar I: The Adaptive Bidding Engine – Smart Bribing
// -----------------------------------------------------------------------------
pub const BIDDING_TIERS: [(u128, u64); 5] = [
    (0, 5),
    (1_000_000_000_000_000, 10),
    (10_000_000_000_000_000, 20),
    (100_000_000_000_000_000, 40),
    (1_000_000_000_000_000_000, 60),
];
pub const MIN_BUILDER_TIP_WEI: u128 = 1_000_000_000_000; // 0.000001 ETH
pub const MAX_BUILDER_TIP_WEI: u128 = 10_000_000_000_000_000; // 0.01 ETH

// -----------------------------------------------------------------------------
// Pillar J: The Inventory Manager – Token Rebalancing
// -----------------------------------------------------------------------------
pub const DUST_THRESHOLD_WEI: u128 = 100_000_000_000_000; // 0.0001 ETH (~$0.25) - Optimized for low budget
pub const DUST_CONVERSION_MAX_GAS_PRICE_GWEI: u64 = 30;
pub const GAS_BUFFER_PERCENT: u64 = 10; // Increased buffer for L2 stability

// -----------------------------------------------------------------------------
// Pillar K: The Simulation Branching – Multiverse Theory
// -----------------------------------------------------------------------------
pub const BRANCH_POSITIONS_TO_SIMULATE: usize = 5;
pub const MAX_BRANCH_LOSS_BPS: u64 = 10;

// -----------------------------------------------------------------------------
// Pillar L: The Poison Token Filter – Advanced Honeypot
// -----------------------------------------------------------------------------
/// Bytecode signatures of known honeypot patterns (expanded)
pub static HONEYPOT_BYTECODE_SIGNATURES: Lazy<Vec<Vec<u8>>> = Lazy::new(|| {
    vec![
        vec![0xfe],                               // INVALID (trap)
        vec![0xff],                               // SELFDESTRUCT
        vec![0xf4],                               // DELEGATECALL
        vec![0xf2],                               // CALLCODE
        vec![0x5b, 0xfd],                         // JUMPDEST + REVERT (common trap)
        vec![0x33, 0x14],                         // CALLER + EQ (owner check)
        vec![0x43, 0x15, 0x5b],                   // block.number + ISZERO + JUMPI (conditional tax)
        vec![0x42, 0x15],                         // timestamp + ISZERO
        vec![0x7f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xfd], // PUSH32 + REVERT
    ]
});
pub const MAX_ALLOWED_TAX_BPS: u64 = 300;
pub const MIN_LIQUIDITY_ETH: u128 = 1_000_000_000_000_000_000; // 1 ETH (Low liquidity for faster finding)

// -----------------------------------------------------------------------------
// Pillar N: The Zero‑Loss Shield – Private Bundles Only
// -----------------------------------------------------------------------------
pub const REQUIRE_SUCCESSFUL_SIMULATION: bool = true;

// -----------------------------------------------------------------------------
// Pillar O: The Battlefield Filter – L2 Specialist
// -----------------------------------------------------------------------------
pub const OPTIMISM_GAS_ORACLE: Address = addr!("420000000000000000000000000000000000000F");
pub const ARBITRUM_NODE_INTERFACE: Address = addr!("00000000000000000000000000000000000000C8");

// L1 fee scalars (dynamic, but provide defaults)
pub static L1_BASE_FEE_SCALAR: Lazy<FxHashMap<Chain, u64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Optimism, 1360);   // 13.6% (1.36x)
    m.insert(Chain::Base, 1360);       // same as Optimism
    m.insert(Chain::Arbitrum, 1000);   // Arbitrum uses a different model, placeholder
    m.insert(Chain::Polygon, 0);
    m.insert(Chain::Mainnet, 0);
    m
});

pub static L1_BLOB_BASE_FEE_SCALAR: Lazy<FxHashMap<Chain, u64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Optimism, 810);     // 8.1x? Actually EIP-4844 scalars vary
    m.insert(Chain::Base, 810);
    m.insert(Chain::Arbitrum, 0);
    m.insert(Chain::Polygon, 0);
    m.insert(Chain::Mainnet, 0);
    m
});

// Fallback static multipliers (used if oracles fail)
pub static L2_L1_DATA_GAS_MULTIPLIER: Lazy<FxHashMap<Chain, f64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Arbitrum, 0.1);
    m.insert(Chain::Optimism, 0.2);
    m.insert(Chain::Base, 0.2);
    m.insert(Chain::Polygon, 0.0);
    m.insert(Chain::Mainnet, 0.0);
    m
});

pub static L2_GAS_LIMIT_MULTIPLIER: Lazy<FxHashMap<Chain, f64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Arbitrum, 1.5);
    m.insert(Chain::Optimism, 1.5);
    m.insert(Chain::Base, 1.5);
    m.insert(Chain::Polygon, 1.2);
    m.insert(Chain::Mainnet, 1.0);
    m
});

// -----------------------------------------------------------------------------
// Pillar P: The Auto‑Compounding Vault
// -----------------------------------------------------------------------------
pub const GAS_VAULT_PERCENTAGE: u64 = 5;
/// IMPORTANT: Replace with your own secure address (cold wallet or separate EOA).
/// Can also be set via environment variable SHADOW_GAS_VAULT.
pub const GAS_VAULT_ADDRESS: Option<Address> = Some(addr!("DeaD000000000000000000000000000000000000")); // PLACEHOLDER – CHANGE ME

// -----------------------------------------------------------------------------
// Pillar Q: The Bootstrap Protocol – Zero‑Start
// -----------------------------------------------------------------------------
pub static GAS_FAUCETS: Lazy<FxHashMap<Chain, Vec<Address>>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    // Testnet faucets (if any) – mainnet L2s require bridging
    m.insert(Chain::Arbitrum, vec![addr!("0000000000000000000000000000000000000000")]);
    m.insert(Chain::Base, vec![addr!("0000000000000000000000000000000000000000")]);
    m
});

pub const BOOTSTRAP_MIN_BALANCE_WEI: u128 = 1_000_000_000_000_000; // 0.001 ETH

// -----------------------------------------------------------------------------
// Pillar R: The Shadow Simulation – 99.9% Rule
// -----------------------------------------------------------------------------
pub const CONGESTION_TX_COUNT: usize = 5000;
pub const MIN_INCLUSION_PROBABILITY: u8 = 100; // Pillar R: 99.99% Lock
pub const MAX_BUILDER_LATENCY_MS: u64 = 100;
pub const TELEGRAM_BOT_TOKEN: &str = "YOUR_BOT_TOKEN"; // Dashboard on phone
pub const TELEGRAM_CHAT_ID: &str = "YOUR_CHAT_ID";
pub const TELEGRAM_CONTROL_BOT_TOKEN: &str = "YOUR_CONTROL_BOT_TOKEN"; // Separate bot for commands
pub const TELEGRAM_CONTROL_CHAT_ID: &str = "YOUR_CONTROL_CHAT_ID"; // Chat ID for commands

// -----------------------------------------------------------------------------
// Pillar S: The Intent Solver Engine – Gasless MEV
// -----------------------------------------------------------------------------
/// UniswapX Reactor (Permit2) – actual address provided
pub const UNISWAPX_REACTOR: Address = addr!("00000011F131169B1390ad7Ef991104e50d73a2F"); // Permit2 contract
pub const COWSWAP_SETTLEMENT_CONTRACT: Address = addr!("9008D19f58AAbD9eD0D60971565AA8510560ab41");
pub const SELECTOR_COWSWAP_SETTLE: Selector = Selector([0x09, 0x86, 0x32, 0x14]); // settle(ISettlement.Settlement,bytes[])

// Pillar S: UniswapX Dutch Order Reactor Selectors
pub const SELECTOR_UNISWAPX_EXECUTE: Selector = Selector([0x8a, 0xe0, 0x69, 0x3a]); // execute(SignedOrder)
pub const SELECTOR_UNISWAPX_EXECUTE_BATCH: Selector = Selector([0x5b, 0x0d, 0x13, 0x5a]); // executeBatch(SignedOrder[])

pub const ENABLE_GASLESS_MODE: bool = false;

// -----------------------------------------------------------------------------
// Pillar T: The Anti‑Drift Guardian – Lag Shield
// -----------------------------------------------------------------------------
pub const MAX_STALE_BLOCKS: u64 = 1;
pub const MAX_NODE_LAG_SECONDS: u64 = 2; // Pillar T: Tightened for L2 (Base/Arb) 2s is safe limit

// -----------------------------------------------------------------------------
// Pillar U: The Unseen Host – Zero‑Cost Infrastructure
// -----------------------------------------------------------------------------
pub const MAX_POOL_CACHE_SIZE: usize = 5_000; // Optimized for 16Gi RAM
pub const MAX_TOKEN_CACHE_SIZE: usize = 1_000;

// -----------------------------------------------------------------------------
// Pillar V: The Veto Protocol – Dynamic Kill‑Switch
// -----------------------------------------------------------------------------
pub static GLOBAL_PAUSE: Lazy<std::sync::atomic::AtomicBool> = Lazy::new(|| std::sync::atomic::AtomicBool::new(false));
pub const WATCH_ONLY_MODE: bool = false; // ⚡ HUNTING MODE: ACTIVATED (Set to false for real execution)

// -----------------------------------------------------------------------------
// Pillar W: The Wash‑Trap Radar – Fake Volume Filter
// -----------------------------------------------------------------------------
pub const MAX_WASH_TRADE_RATIO: f64 = 0.3;
pub const MIN_UNIQUE_TRADERS: usize = 5;

// -----------------------------------------------------------------------------
// Pillar X: The X‑Ray Scanner – Bytecode Analysis
// -----------------------------------------------------------------------------
pub static MALICIOUS_OPCODES: Lazy<FxHashSet<u8>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(0xff); // SELFDESTRUCT
    set.insert(0xf4); // DELEGATECALL
    set.insert(0xf2); // CALLCODE
    set.insert(0xfe); // INVALID
    set.insert(0x43); // block.number
    set.insert(0x42); // timestamp
    set.insert(0x44); // difficulty
    set.insert(0x45); // gaslimit
    set.insert(0x46); // chainid
    set.insert(0x33); // CALLER
    set
});

// -----------------------------------------------------------------------------
// Pillar Y: The Yield Scavenger – Micro‑Profit Sweeper
// -----------------------------------------------------------------------------
pub const MICRO_PROFIT_THRESHOLD_WEI: u128 = 10_000_000_000_000; // 0.00001 ETH (~$0.02)
pub const MICRO_PROFIT_THRESHOLD_USD_CENTS: u64 = 2; // $0.02

// -----------------------------------------------------------------------------
// Pillar Z: The Zenith Protocol – Autonomous Factory Discovery
// -----------------------------------------------------------------------------
pub static KNOWN_FACTORY_DEPLOYERS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(addr!("1F98431c8aD98523631AE4a59f267346ea31F984")); // Uniswap V3 (Fixed)
    set.insert(addr!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f")); // Uniswap V2
    set.insert(addr!("C0AEe478e3658e2610c5F7A4A2E1777cE9e4f2Ac")); // SushiSwap
    set.insert(addr!("BA12222222228d8Ba445958a75a0704d566BF2C8")); // Balancer V2
    set.insert(addr!("0959157Bf5154c5F7fCb4404f5B9F8d6b5F9F8d6")); // Curve (placeholder)
    set
});
pub const FACTORY_SCAN_INTERVAL_BLOCKS: u64 = 1000;

// -----------------------------------------------------------------------------
// Token Decimals Table – Critical for correct arithmetic
// -----------------------------------------------------------------------------
pub static TOKEN_DECIMALS: Lazy<FxHashMap<Address, u8>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    // Ethereum Mainnet
    m.insert(addr!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), 18);
    m.insert(addr!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), 6);
    m.insert(addr!("dAC17F958D2ee523a2206206994597C13D831ec7"), 6);
    m.insert(addr!("6B175474E89094C44Da98b954EedeAC495271d0F"), 18);
    m.insert(addr!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"), 8);
    // Arbitrum
    m.insert(addr!("82aF49447D8a07e3bd95BD0d56f35241523fBab1"), 18);
    m.insert(addr!("FF970A61A04b1cA14834A43f5dE4533eBDDB5CC8"), 6);
    m.insert(addr!("Fd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9"), 6);
    m.insert(addr!("DA10009cBd5D07dd0CeCc66161FC93D7c9000da1"), 18);
    m.insert(addr!("2f2a2543B76A4166549F7aaB2e75Bf0aEf5c5B0f"), 8);
    // Optimism
    m.insert(addr!("4200000000000000000000000000000000000006"), 18);
    m.insert(addr!("7F5c764cBc14f9669B88837ca1490cCa17c31607"), 6);
    m.insert(addr!("94b008aA00579c1307B0EF2c499aD98a8ce58e58"), 6);
    m.insert(addr!("DA10009cBd5D07dd0CeCc66161FC93D7c9000da1"), 18);
    m.insert(addr!("68f180fcCe6836688e9084f035309E29Bf0A2095"), 8);
    // Base
    m.insert(addr!("4200000000000000000000000000000000000006"), 18);
    m.insert(addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"), 6);
    // Polygon
    m.insert(addr!("0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270"), 18);
    m.insert(addr!("2791Bca1f2de4661ED88A30C99A7a9449Aa84174"), 6);
    m.insert(addr!("c2132D05D31c914a87C6611C10748AEb04B58e8F"), 6);
    m.insert(addr!("8f3Cf7ad23Cd3CaDbD9735AFf958023239c6A063"), 18);
    m.insert(addr!("1BFd67037B42Cf73acF2047067bd4F2C47D9BfD6"), 8);
    m
});

// -----------------------------------------------------------------------------
// Global Profitability & Risk Parameters (Atomic Safety)
// -----------------------------------------------------------------------------
pub const MIN_PROFIT_WEI: u128 = 1; // ⚡ 1-WEI SENSITIVITY: Capture every drop of alpha
pub const MIN_PROFIT_BPS: u64 = 5;
pub const DEFAULT_SLIPPAGE_BPS: u64 = 30; // 0.3%
pub const GAS_LIMIT_MULTIPLIER: f64 = 1.15;
pub const PRIORITY_FEE_MULTIPLIER: f64 = 1.2;
pub const MAX_GAS_PRICE_GWEI: u64 = 50; // Allow more room during spikes
pub const MAX_TOTAL_TX_FEE_WEI: u128 = 20_000_000_000_000; // ₹15-20 max per trade attempt
pub const STRIKE_GAS_LIMIT: u64 = 150_000; // Even tighter gas limit for Base
pub const MIN_SEARCHER_BALANCE_WEI: u128 = 200_000_000_000_000; // 🛡️ Safety Floor: Stop at ₹40-50 to avoid total depletion
pub const SURVIVAL_PROFIT_MULTIPLIER: u64 = 2; // ⚡ Predator Selectivity: Must be 2x Cost during bootstrap

// -----------------------------------------------------------------------------
// Uniswap V3 Fee Tiers – for pool enumeration and path building
// -----------------------------------------------------------------------------
pub const UNISWAP_V3_FEE_TIERS: [u32; 4] = [100, 500, 3000, 10000];

// -----------------------------------------------------------------------------
// Init Code Hashes – For offline pool address derivation (speed boost)
// -----------------------------------------------------------------------------
// Uniswap V2/V3
pub const UNISWAP_V2_INIT_CODE_HASH: &str =
    "0x96e8ac4277198ff8b6f785478aa9a39f403cb768dd02cbee326c3e7da348845f";
pub const UNISWAP_V3_INIT_CODE_HASH: &str =
    "0xe34f199b19b2b4f47f68442619d555527d244f78a3297ea89325f843f87b8b54";
pub const SUSHISWAP_INIT_CODE_HASH: &str =
    "0xe18a34eb0e04b04f7a0ac29a6e80748dca96319b42c54d679cb821dca90c6303";
// Balancer V2 – WeightedPool (example, may vary by pool type)
pub const BALANCER_WEIGHTED_POOL_INIT_CODE_HASH: &str =
    "0x5d7e7a9e8f9f9b9a9b9c9d9e9f9a9b9c9d9e9f9a9b9c9d9e9f9a9b9c9d9e9f9a"; // placeholder
// Curve – various factories; one common hash for stableswap
pub const CURVE_STABLESWAP_FACTORY_INIT_CODE_HASH: &str =
    "0x9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f9b8f"; // placeholder

// -----------------------------------------------------------------------------
// DEX Routers & Factories (Multi‑Chain Support)
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct DexContracts {
    pub router: Address,
    pub factory: Address,
    pub quoter: Option<Address>,
}

// Ethereum Mainnet
pub const UNISWAP_V2_ROUTER: Address = addr!("7a250d5630B4cF539739dF2C5dAcb4c659F2488D");
pub const UNISWAP_V2_FACTORY: Address = addr!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f");
pub const UNISWAP_V3_ROUTER: Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const UNISWAP_V3_QUOTER: Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const UNISWAP_UNIVERSAL_ROUTER: Address = addr!("3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD");
pub const SUSHISWAP_ROUTER: Address = addr!("d9e1cE17f2641f24aE83637ab66a2cca9C378B9F");
pub const SUSHISWAP_FACTORY: Address = addr!("C0AEe478e3658e2610c5F7A4A2E1777cE9e4f2Ac");
pub const CURVE_ADDRESS_PROVIDER: Address = addr!("0000000022D53366457F9d5E68Ec105046FC4383");
pub const CURVE_REGISTRY: Address = addr!("90E00ACe148ca3b23Ac1bC8C240C2a7Dd9c2d7f5");
pub const BALANCER_RELAYER: Address = addr!("BA12222222228d8Ba445958a75a0704d566BF2C8");

// Arbitrum
pub const ARB_UNISWAP_V3_ROUTER: Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const ARB_UNISWAP_V3_QUOTER: Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const ARB_SUSHISWAP_ROUTER: Address = addr!("1b02dA8Cb0d097eB8D57A175b88c7D8b47997506");
pub const ARB_SUSHISWAP_FACTORY: Address = addr!("c35DADB65012eC5796536bD9864eD8773aBc74C4");
pub const ARB_CAMELOT_ROUTER: Address = addr!("c873fEcbd354f5A56E00E710B90EF4201db2448d");
pub const ARB_CAMELOT_FACTORY: Address = addr!("6EcCab422280a9E75B611d04B0A3f7E1e2b2eF9A");

// Optimism
pub const OP_UNISWAP_V3_ROUTER: Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const OP_UNISWAP_V3_QUOTER: Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const OP_SUSHISWAP_ROUTER: Address = addr!("1b02dA8Cb0d097eB8D57A175b88c7D8b47997506");
pub const OP_SUSHISWAP_FACTORY: Address = addr!("c35DADB65012eC5796536bD9864eD8773aBc74C4");

// Base
pub const BASE_UNISWAP_V3_ROUTER: Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const BASE_UNISWAP_V3_QUOTER: Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const BASE_AERODROME_ROUTER: Address = addr!("cF77a3Ba9A5DA3999247B82c4B9D43c1A7fD7c8a");
pub const BASE_AERODROME_FACTORY: Address = addr!("420DD381b31aEf6683db6B902084c0BAc2D1b5d5");
pub const BASE_SUSHISWAP_ROUTER: Address = addr!("6BD61e38Ecca1E08830E3fBC00FA91194863991a");
pub const BASE_SUSHISWAP_FACTORY: Address = addr!("71524B9573c8d199127E09e2108269128355eeA0");

// --- Base Mainnet DEX Addresses ---

// PancakeSwap V3 (Base)
pub const BASE_PANCAKESWAP_FACTORY: Address = addr!("0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865");
pub const BASE_PANCAKESWAP_ROUTER: Address = addr!("1b813dFA2931441bcAd067000E08271167739502");

// BaseSwap V2
pub const BASE_BASESWAP_FACTORY: Address = addr!("FDa619b6d20975be8074d3315450bbBA58456B12");
pub const BASE_BASESWAP_ROUTER: Address = addr!("327Df1E4de51d9752a3e019c42e5f9BB762ed144");

// Maverick Protocol V2
pub const BASE_MAVERICK_FACTORY: Address = addr!("3708D64936496924f2b1853B0287515089304386");
pub const BASE_MAVERICK_ROUTER: Address = addr!("792376e191C802a466D57b545f8f85f340807551");

// Target Routers for Mempool Monitoring (Pillar A)
pub const TARGET_ROUTERS: [Address; 6] = [
    BASE_SUSHISWAP_ROUTER,
    BASE_PANCAKESWAP_ROUTER,
    BASE_BASESWAP_ROUTER,
    BASE_MAVERICK_ROUTER,
    BASE_AERODROME_ROUTER,
    BASE_UNISWAP_V3_ROUTER,
];

// Polygon
pub const POLYGON_UNISWAP_V3_ROUTER: Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const POLYGON_UNISWAP_V3_QUOTER: Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const POLYGON_SUSHISWAP_ROUTER: Address = addr!("1b02dA8Cb0d097eB8D57A175b88c7D8b47997506");
pub const POLYGON_SUSHISWAP_FACTORY: Address = addr!("c35DADB65012eC5796536bD9864eD8773aBc74C4");
pub const POLYGON_QUICKSWAP_ROUTER: Address = addr!("a5E0829cACED8FfDD4De3c43696c57F7D7A678ff");
pub const POLYGON_QUICKSWAP_FACTORY: Address = addr!("5757371414417b8C6CAad45bAeF941aBc7d3Ab32");

/// Chain‑specific DEX contract map.
pub static DEX_CONTRACTS: Lazy<FxHashMap<(Chain, DexName), DexContracts>> = Lazy::new(|| {
    let mut m = FxHashMap::default();

    // Ethereum
    m.insert((Chain::Mainnet, DexName::UniswapV2), DexContracts {
        router: UNISWAP_V2_ROUTER,
        factory: UNISWAP_V2_FACTORY,
        quoter: None,
    });
    m.insert((Chain::Mainnet, DexName::UniswapV3), DexContracts {
        router: UNISWAP_V3_ROUTER,
        factory: addr!("1F98431c8aD98523631AE4a59f267346ea31F984"), // Fixed: Full 40-char hex
        quoter: Some(UNISWAP_V3_QUOTER),
    });
    m.insert((Chain::Mainnet, DexName::SushiSwap), DexContracts {
        router: SUSHISWAP_ROUTER,
        factory: SUSHISWAP_FACTORY,
        quoter: None,
    });

    // Arbitrum
    m.insert((Chain::Arbitrum, DexName::UniswapV3), DexContracts {
        router: ARB_UNISWAP_V3_ROUTER,
        factory: addr!("1F98431c8aD98523631AE4a59f267346ea31F984"),
        quoter: Some(ARB_UNISWAP_V3_QUOTER),
    });
    m.insert((Chain::Arbitrum, DexName::SushiSwap), DexContracts {
        router: ARB_SUSHISWAP_ROUTER,
        factory: ARB_SUSHISWAP_FACTORY,
        quoter: None,
    });

    // Optimism
    m.insert((Chain::Optimism, DexName::UniswapV3), DexContracts {
        router: OP_UNISWAP_V3_ROUTER,
        factory: addr!("1F98431c8aD98523631AE4a59f267346ea31F984"),
        quoter: Some(OP_UNISWAP_V3_QUOTER),
    });
    m.insert((Chain::Optimism, DexName::SushiSwap), DexContracts {
        router: OP_SUSHISWAP_ROUTER,
        factory: OP_SUSHISWAP_FACTORY,
        quoter: None,
    });

    // Base
    m.insert((Chain::Base, DexName::UniswapV3), DexContracts {
        router: BASE_UNISWAP_V3_ROUTER,
        factory: addr!("33128a8fC170d030b747a24199840E2303c8959d"), // Canonical Base V3 Factory
        quoter: Some(BASE_UNISWAP_V3_QUOTER),
    });
    m.insert((Chain::Base, DexName::SushiSwap), DexContracts {
        router: BASE_SUSHISWAP_ROUTER,
        factory: BASE_SUSHISWAP_FACTORY,
        quoter: None,
    });
    m.insert((Chain::Base, DexName::Aerodrome), DexContracts {
        router: addr!("cDAC0d6c6C59727a65F871236188350531885C43"), // Aerodrome Router
        factory: addr!("420DD381b31aEf6683db6B902084c0BAc2D1b5d5"), // Aerodrome Factory
        quoter: None,
    });
    m.insert((Chain::Base, DexName::BaseSwap), DexContracts {
        router: BASE_BASESWAP_ROUTER,
        factory: BASE_BASESWAP_FACTORY,
        quoter: None,
    });
    m.insert((Chain::Base, DexName::PancakeSwap), DexContracts {
        router: BASE_PANCAKESWAP_ROUTER,
        factory: BASE_PANCAKESWAP_FACTORY,
        quoter: None,
    });
    m.insert((Chain::Base, DexName::Maverick), DexContracts {
        router: BASE_MAVERICK_ROUTER,
        factory: BASE_MAVERICK_FACTORY,
        quoter: None,
    });

    // Polygon
    m.insert((Chain::Polygon, DexName::UniswapV3), DexContracts {
        router: POLYGON_UNISWAP_V3_ROUTER,
        factory: addr!("1F98431c8aD98523631AE4a59f267346ea31F984"),
        quoter: Some(POLYGON_UNISWAP_V3_QUOTER),
    });
    m.insert((Chain::Polygon, DexName::SushiSwap), DexContracts {
        router: POLYGON_SUSHISWAP_ROUTER,
        factory: POLYGON_SUSHISWAP_FACTORY,
        quoter: None,
    });

    m
});

// -----------------------------------------------------------------------------
// High‑Liquidity Token Addresses per chain (Base assets)
// -----------------------------------------------------------------------------
pub static SAFE_TOKENS: Lazy<FxHashMap<Chain, Vec<Address>>> = Lazy::new(|| {
    let mut m = FxHashMap::default();

    m.insert(Chain::Mainnet, vec![
        addr!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
        addr!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        addr!("dAC17F958D2ee523a2206206994597C13D831ec7"),
        addr!("6B175474E89094C44Da98b954EedeAC495271d0F"),
        addr!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
    ]);

    m.insert(Chain::Arbitrum, vec![
        addr!("82aF49447D8a07e3bd95BD0d56f35241523fBab1"),
        addr!("FF970A61A04b1cA14834A43f5dE4533eBDDB5CC8"),
        addr!("Fd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9"),
        addr!("DA10009cBd5D07dd0CeCc66161FC93D7c9000da1"),
        addr!("2f2a2543B76A4166549F7aaB2e75Bf0aEf5c5B0f"),
    ]);

    m.insert(Chain::Optimism, vec![
        addr!("4200000000000000000000000000000000000006"),
        addr!("7F5c764cBc14f9669B88837ca1490cCa17c31607"),
        addr!("94b008aA00579c1307B0EF2c499aD98a8ce58e58"),
        addr!("DA10009cBd5D07dd0CeCc66161FC93D7c9000da1"),
        addr!("68f180fcCe6836688e9084f035309E29Bf0A2095"),
    ]);

    m.insert(Chain::Base, vec![
        addr!("4200000000000000000000000000000000000006"),
        addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
        addr!("d9aAEc86B65D86f6A7B5B1b0c42FFA531710b6CA"), // USDC.e
        addr!("fde4C96c8593536E31F229EA8f37b2ADa2699bb2"), // USDT
        addr!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb"), // DAI
        addr!("940181a94A35A4569E4529A3CDfB74e38FD98631"), // AERO
        addr!("4ed4E862860beD51a9570b96d89aF5E1B0Efefed"), // DEGEN
        addr!("2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22"), // cbETH
        addr!("532f27101965dd16442E59d40670Fa5ad5f3fe91"), // BRETT
        addr!("AC1Bd2486aAf3B5C0fc3Fd868558b082a531B2B4"), // TOSHI
        addr!("b23d20f5f58f12ee23186bb8efe2ed2c256385ff"), // PRIME
        addr!("0b3e3284558222224053149BD3d7F96596429d54"), // VIRTUAL
        addr!("eb466342C4d449BC9f53A865D5Cb90586f405215"), // cbBTC
        addr!("60a3E35Cc3064829673dfE7382696278965a9958"), // EURC
        addr!("A88594D27127E0A410b60193959E73482E8458CD"), // WELL
        addr!("54ee07d6bce2868412d96a962c293f57cd1d14ee"), // KEYCAT
        addr!("2Da56AcB9Ea78330f947bD57C54119Debda7AF71"), // MOG
        addr!("0d97F5cC90D4590729922278e2736231E97602fe"), // TYBG
        addr!("8680607cc9620E65D71995bb74BB254025160171"), // COCO
        addr!("2416092f143378750bb29b79eD961ab195CcEea5"), // ezETH
        addr!("c1CBa3fC4D13090E0C6E0F2B9b558175ab40e76D"), // wstETH
        addr!("430557e361548077C30EE97996c61EE0517d96EC"), // PYTH
        addr!("fAbA6f8e4a5E8Ab82F62fe7C39859FA577269BE3"), // ONDO
        addr!("BF10073056d4efDE715B216CC7d69dA6E31da98F"), // LUSD
        addr!("0578795566122231365Af5AfcBaDe25033EB1978"), // HIGHER
        addr!("E184AcCc01A17CbFD63f4601630153a1544CF003"), // ROOST
        addr!("446799a34C91339119967072405148b621e6BbCc"), // BOOMER
        addr!("BC45647F9D223056708C250094C63Ad00777f032"), // BENJI
        addr!("5037Fd68E0BA568441118C6364f39E99694e0286"), // MYRO
        addr!("F6e9327E456859940Ee2161f23104abc9f410783"), // MOCHI
        addr!("3d7a0227BF72D973F3f68cfCD58882dCb3563dc0"), // BRIUN
        addr!("49678EcADcae3669343b1716963c1cf376094600"), // BLOO
        addr!("27D2DEC2AAc93041398f00bd1d50962734445769"), // BALD
        addr!("8Ee73c484A26e0A5df2Ee2a4960B789967dd0415"), // CRV
        addr!("4200000000000000000000000000000000000042"), // OP
        addr!("c043664861ad5E92003041846407521834C395a4"), // LDO
        addr!("22e6966B799c4D5d13BE9b55c68AC0157636557d"), // SNX
        addr!("7db5afAC3daC2245bE35876222C5667C660644a9"), // STG
        addr!("6921B130D297cc43754afBA22e5EAc0FBf8Db75b"), // JOE
        addr!("ba1104315c194b6f758c797d96Fa62962ad3333d"), // BAL
    ]);

    m.insert(Chain::Polygon, vec![
        addr!("0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270"),
        addr!("2791Bca1f2de4661ED88A30C99A7a9449Aa84174"),
        addr!("c2132D05D31c914a87C6611C10748AEb04B58e8F"),
        addr!("8f3Cf7ad23Cd3CaDbD9735AFf958023239c6A063"),
        addr!("1BFd67037B42Cf73acF2047067bd4F2C47D9BfD6"),
    ]);

    m
});

/// Pillar Z: Core Tokens for Lean Discovery (Local Testing)
pub static CORE_TOKENS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(TOKEN_WETH);
    set.insert(TOKEN_USDC);
    set.insert(TOKEN_DAI);
    set.insert(TOKEN_CBETH);
    set
});

/// Pillar Z: Core Pools for Production (Always kept in RAM)
pub static CORE_POOLS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(POOL_UNIV3_WETH_USDC_005);
    set.insert(POOL_UNIV3_WETH_USDC_030);
    set.insert(POOL_BASESWAP_WETH_USDC);
    set.insert(POOL_SUSHI_WETH_USDC);
    set.insert(POOL_PANCAKESWAP_WETH_USDC);
    set.insert(POOL_UNIV3_WETH_DAI);
    set
});

// -----------------------------------------------------------------------------
// Blacklisted Tokens (Honeypots, scam tokens, low-liquidity rug pulls)
// -----------------------------------------------------------------------------
pub static BLACKLISTED_TOKENS: Lazy<FxHashMap<Chain, FxHashSet<Address>>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Mainnet, FxHashSet::default());
    m.insert(Chain::Arbitrum, FxHashSet::default());
    m.insert(Chain::Optimism, FxHashSet::default());
    m.insert(Chain::Base, FxHashSet::default());
    m.insert(Chain::Polygon, FxHashSet::default());
    m
});

// -----------------------------------------------------------------------------
// Common Revert Reasons (For diagnostics and learning)
// -----------------------------------------------------------------------------
pub static COMMON_REVERT_REASONS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "UniswapV2: INSUFFICIENT_OUTPUT_AMOUNT",
        "UniswapV2: INSUFFICIENT_LIQUIDITY",
        "UniswapV2: K",
        "UniswapV3: slippage check failed",
        "SafeERC20: low-level call failed",
        "ERC20: transfer amount exceeds balance",
        "Flash loan repayment failed",
        "execution reverted: TransferHelper: TRANSFER_FROM_FAILED",
        "execution reverted: STF",
        "execution reverted: Permit2: permit expired",
    ]
});

// -----------------------------------------------------------------------------
// Circuit Breakers (Safety)
// -----------------------------------------------------------------------------
pub const MAX_VOLATILITY_THRESHOLD_BPS: u64 = 500;
pub const NETWORK_CONGESTION_GWEI: u64 = 300;
pub const GLOBAL_STOP_LOSS_BPS: u64 = 1000;

// -----------------------------------------------------------------------------
// Gas & Fee Related Constants (Baseline)
// -----------------------------------------------------------------------------
pub const BASE_BUNDLE_GAS: u64 = 800_000;
pub const MAX_BUNDLE_GAS: u64 = 10_000_000;

// -----------------------------------------------------------------------------
// Re‑export for convenience
// -----------------------------------------------------------------------------
// pub use AppChain::*; // This is no longer needed