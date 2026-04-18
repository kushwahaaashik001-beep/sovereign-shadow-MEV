use alloy_primitives::Address;
use once_cell::sync::Lazy;
use rustc_hash::{FxHashMap, FxHashSet};
use crate::models::{Chain, DexName, Selector};

macro_rules! addr {
    ($s:expr) => { alloy_primitives::address!($s) };
}

pub const ENV_RPC_URL: &str = "SHADOW_RPC_URL";
pub const ENV_WS_URL: &str = "SHADOW_WS_URL";
pub const ENV_HTTP_URL: &str = "SHADOW_HTTP_URL";
pub const ENV_PRIVATE_KEY: &str = "SHADOW_PRIVATE_KEY";
pub const ENV_FLASHBOTS_RELAY: &str = "SHADOW_FLASHBOTS_RELAY";

pub const ENV_GAS_VAULT_ADDRESS: &str = "SHADOW_GAS_VAULT";
pub const ENV_EXECUTOR_ADDRESS: &str = "SHADOW_EXECUTOR_ADDRESS";

pub const TOKEN_AERO:   Address = addr!("940181a94A35A4569E4529A3CDfB74e38FD98631");
pub const TOKEN_CBETH:  Address = addr!("2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22");
pub const TOKEN_DAI:    Address = addr!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb");
pub const TOKEN_DEGEN:  Address = addr!("4ed4E862860beD51a9570b96d89aF5E1B0Efefed");
pub const TOKEN_USDT:   Address = addr!("fde4C96c8593536E31F229EA8f37b2ADa2699bb2");
pub const TOKEN_BRETT:  Address = addr!("532f27101965dd16442E59d40670Fa5ad5f3fe91");
pub const TOKEN_WETH:   Address = addr!("4200000000000000000000000000000000000006");
pub const TOKEN_USDC:   Address = addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
pub const TOKEN_VIRTUAL: Address = addr!("0x0bFbCF9fa4f9C56B0F40a671Ad40E0805A091865");
pub const TOKEN_AIXBT:   Address = addr!("0x4F9Fd6Be4a90f2620860d680c0d4d5Fb53d1A84E");
pub const TOKEN_HIGHER:  Address = addr!("0x057871A20512f5343046bc3A531557d903698064");
pub const TOKEN_LUNA:    Address = addr!("0xD533a949740bb3306d119CC777fa900bA034cd52");
pub const TOKEN_MOXIE:   Address = addr!("0x8E903254580327429402f0D4f907693998D654b9");
pub const TOKEN_AI16Z:   Address = addr!("0x30c90069678174577B0Ac49969D7070F7915B597");

pub const POOL_UNIV3_WETH_USDC_005: Address = addr!("d0b53D9277642d899DF5C87A3966A349A798F224");
pub const POOL_UNIV3_WETH_USDC_030: Address = addr!("4C36388bE6F416A29C8d8Eee819bb35ed3737a01");
pub const POOL_BASESWAP_WETH_USDC:  Address = addr!("7E3411B04766089cFaa52DB688855356A12f05D1");
pub const POOL_SUSHI_WETH_USDC:     Address = addr!("21943f679eD5f05329883584860D49C039237685");
pub const POOL_PANCAKESWAP_WETH_USDC: Address = addr!("cDAC0d6c6C59727a65F871236188350531885C43");
pub const POOL_UNIV3_WETH_DAI:      Address = addr!("D9e885B1e6a6B8f7FE1e6E9B5e5e5e5e5e5e5e5e");
pub const POOL_UNIV2_WETH_USDC:     Address = POOL_BASESWAP_WETH_USDC;
pub const POOL_UNIV2_WETH_DEGEN:    Address = addr!("c9034c3E7242654fd148e934814510d0e9436db4");
pub const POOL_UNIV2_USDC_DEGEN:    Address = addr!("4b0Aaf3EBb163dd45F663b38b6d93f6093EBC2d3");
pub const POOL_SUSHI_WETH_DEGEN:    Address = addr!("c9034c3E7242654fd148e934814510d0e9436db4");
pub const POOL_AERO_WETH_USDC:      Address = addr!("cDAC0d6c6C59727a65F871236188350531885C43");
pub const POOL_AERO_WETH_AERO:      Address = addr!("7f670f78B17dEC44d5Ef68a48740b6f8849cc2e6");
pub const POOL_AERO_WETH_BRETT:     Address = addr!("532f27101965dd16442E59d40670Fa5ad5f3fe91");

pub const MOONWELL_COMPTROLLER:     Address = addr!("BBBBBbbBBb9cC5e90e3b3Af64bdAF62C37EEFFCb");
pub const MIN_NET_PROFIT_USD_WEI:   u128 = 60_000_000_000_000; // Scavenger: $0.15 threshold for $40/day goal

// Alpha Hunter: Sweet Spot Liquidity Range ($2k - $30k)
pub const MIN_ALPHA_LIQUIDITY_USD: u128 = 300; // Micro-liquidity capture for stealth arbs
pub const MAX_ALPHA_LIQUIDITY_USD: u128 = 40_000; 
pub const MIN_AI_LIQUIDITY_USD:    u128 = 10_000; // Strict filter for high-risk AI tokens
pub const MAX_AI_TAX_BPS:          u64 = 100;     // 1% max tax for AI cluster
pub const ESTIMATED_ETH_PRICE: u128 = 2_500; // Used for quick in-memory liquidity filtering

pub const POOL_HOTNESS_TTL_SEC: u64 = 1800; // 30 minutes for Autonomous Discovery

/// Pillar Z: Hardcoded Top 100 Pools on Base to save RAM
/// Optimization: Targeting "Alpha Clusters" (Meme & Ecosystem tokens) with lower competition.
pub static TOP_100_POOLS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    // 1. Core Bridges (Zaruri hain rasta banaye rakhne ke liye)
    set.insert(addr!("0xcDAC0d6c6C59727a65F871236188350531885C43")); // Aero WETH/USDC
    set.insert(addr!("0xd0b53D9277642d899DF5C87A3966A349A798F224")); // UniV3 WETH/USDC (0.05%)
    set.insert(addr!("0x4C36388bE6F416A29C8d8Eee819bb35ed3737a01")); // UniV3 WETH/USDC (0.3%)
    set.insert(addr!("0x21943f679eD5f05329883584860D49C039237685")); // Sushi WETH/USDC
    set.insert(addr!("0x7E3411B04766089cFaa52DB688855356A12f05D1")); // BaseSwap WETH/USDC
    
    // 2. Alpha Cluster: DEGEN (Base ka king meme)
    set.insert(addr!("0xc9034c3E7242654fd148e934814510d0e9436db4")); // UniV2 WETH/DEGEN
    set.insert(addr!("0x4b0Aaf3EBb163dd45F663b38b6d93f6093EBC2d3")); // UniV2 USDC/DEGEN
    set.insert(addr!("0x3062ad446da2cfdb10266e06bee30f33ba2a6b41")); // Aero DEGEN/WETH
    set.insert(addr!("0xf0c57173e35181D061033A38166D5726C4A641F2")); // Pancake DEGEN/WETH

    // 3. Alpha Cluster: AERO & Ecosystem
    set.insert(addr!("0x7f670f78B17dEC44d5Ef68a48740b6f8849cc2e6")); // Aero WETH/AERO
    set.insert(addr!("0x532f27101965dd16442E59d40670Fa5ad5f3fe91")); // Aero WETH/BRETT
    set.insert(addr!("0x2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22")); // cbETH/WETH (Price lags often)
    set.insert(addr!("0x420DD381b31aEf6683db6B902084c0BAc2D1b5d5")); // Aero WETH/USDT

    // 4. Alpha Cluster: VIRTUAL & AI Agents
    set.insert(addr!("0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865")); // Pancake WETH/MemeX
    set.insert(addr!("0x0bFbCF9fa4f9C56B0F40a671Ad40E0805A091865")); // Virtual/WETH
    set.insert(addr!("0x30c90069678174577B0Ac49969D7070F7915B597")); // AI16Z/WETH
    
    // 5. Alpha Cluster: The Scavenger Network (Targeting $20-$30/day)
    set.insert(addr!("0x04C9F118A4864700721A163744021d21DB27c11f")); // SwapBased Meme Pair
    set.insert(addr!("0x3D2d7681335A74Be482D207137f814bA688849E8")); // AlienBase Gaming Token
    set.insert(addr!("0x532f27101965dd16442E59d40670Fa5ad5f3fe91")); // Brett Alpha Pair
    set.insert(addr!("0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed")); // Degen Ecosystem Bridge

    set
});

pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_TOKENS: Selector = Selector([0x38, 0xed, 0x17, 0x39]);
pub const SELECTOR_UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_TOKENS: Selector = Selector([0x88, 0x03, 0xdb, 0xee]);
pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_ETH_FOR_TOKENS:    Selector = Selector([0x7f, 0xf3, 0x6a, 0xb5]);
pub const SELECTOR_UNISWAP_V2_SWAP_TOKENS_FOR_EXACT_ETH:    Selector = Selector([0x4a, 0x25, 0xd9, 0x4a]);
pub const SELECTOR_UNISWAP_V2_SWAP_EXACT_TOKENS_FOR_ETH:    Selector = Selector([0x18, 0xcb, 0xaf, 0xe5]);
pub const SELECTOR_UNISWAP_V2_SWAP_ETH_FOR_EXACT_TOKENS:    Selector = Selector([0xfb, 0x3b, 0xdb, 0x41]);
pub const SELECTOR_UNISWAP_V3_EXACT_INPUT:        Selector = Selector([0xb1, 0x58, 0x5b, 0x3f]);
pub const SELECTOR_UNISWAP_V3_EXACT_OUTPUT:       Selector = Selector([0x2a, 0x8e, 0x59, 0x8b]);
pub const SELECTOR_UNISWAP_V3_EXACT_INPUT_SINGLE: Selector = Selector([0x41, 0x4b, 0xf3, 0x89]);
pub const SELECTOR_UNISWAP_V3_EXACT_OUTPUT_SINGLE:Selector = Selector([0xdb, 0x3e, 0x21, 0x98]);
pub const SELECTOR_UNIVERSAL_ROUTER_EXECUTE:      Selector = Selector([0x35, 0x93, 0x56, 0x4c]);
pub const SELECTOR_MULTICALL3:                    Selector = Selector([0xca, 0x02, 0x4c, 0x08]);
pub const SELECTOR_MULTICALL:                     Selector = Selector([0xac, 0x96, 0x50, 0xd8]);
pub const SELECTOR_CURVE_EXCHANGE:                Selector = Selector([0x3d, 0xf0, 0x21, 0x24]);
pub const SELECTOR_CURVE_EXCHANGE_UNDERLYING:     Selector = Selector([0xa6, 0x41, 0x47, 0x5a]);
pub const SELECTOR_CURVE_EXCHANGE_AT_DYNAMIC:     Selector = Selector([0x53, 0xc3, 0x5a, 0x56]);
pub const SELECTOR_CURVE_ADD_LIQUIDITY:           Selector = Selector([0x0b, 0x4c, 0x7e, 0x27]);
pub const SELECTOR_BALANCER_SWAP:                 Selector = Selector([0x52, 0xbb, 0xbe, 0x29]);
pub const SELECTOR_BALANCER_BATCH_SWAP:           Selector = Selector([0x94, 0x5b, 0xce, 0xc9]);
pub const SELECTOR_PERMIT2_PERMIT:                Selector = Selector([0x2b, 0x67, 0x8a, 0x24]);
pub const SELECTOR_PERMIT2_TRANSFER_FROM:         Selector = Selector([0x36, 0x78, 0x00, 0x7b]);
pub const SELECTOR_UPGRADE_TO:                    Selector = Selector([0x36, 0x59, 0xcf, 0xe6]);
pub const SELECTOR_UPGRADE_TO_AND_CALL:           Selector = Selector([0x4f, 0x1e, 0xf3, 0xd0]);
pub const SELECTOR_SET_FEE:                       Selector = Selector([0x1a, 0x6d, 0x05, 0x51]);
pub const SELECTOR_COWSWAP_SETTLE:                Selector = Selector([0x09, 0x86, 0x32, 0x14]);
pub const SELECTOR_UNISWAPX_EXECUTE:              Selector = Selector([0x8a, 0xe0, 0x69, 0x3a]);
pub const SELECTOR_UNISWAPX_EXECUTE_BATCH:        Selector = Selector([0x5b, 0x0d, 0x13, 0x5a]);

// Event Topics for Factory Scanner
pub const EVENT_V2_PAIR_CREATED: [u8; 32] = alloy_primitives::fixed_bytes!("0x0d3648bd0f6ba80134a33ba9275ac585d9d315f0ad835062d573067ad61d5733").0;
pub const EVENT_V3_POOL_CREATED: [u8; 32] = alloy_primitives::fixed_bytes!("0x783cca1c0412dd0d695e784568c96da2e9c22ffc959087dddeaf319803d01584").0;

// Pillar M: Flash Loan Providers (Base Mainnet)
pub const BALANCER_VAULT: Address = addr!("BA12222222228d8Ba445958a75a0704d566BF2C8");
pub const AAVE_V3_POOL:   Address = addr!("A238Dd80C259a72e81d7e4664a9801593F98d1c5");

pub const MAX_TICK_CROSSES: usize = 256;
pub const SIMULATION_GAS_LIMIT: u64 = 600_000;
pub const SIMULATION_VERBOSE: bool = false;
pub const MAX_HOPS: usize = 6; // Alpha Hunter: Deep 6-hop cycles for obscure meme paths
pub const TOP_N_TOKENS: usize = 50;
pub const PATH_CACHE_TTL_MS: u64 = 5000;
pub const GSS_TOLERANCE_WEI: u128 = 1_000_000_000_000;
pub const MIN_OPTIMIZATION_AMOUNT_WEI: u128 = 100_000_000_000_000;
pub const POOL_REPLACEMENT_INTERVAL_SEC: u64 = 300;

pub const FLASHBOTS_RELAY:  &str = "https://relay.flashbots.net";
pub const BEAVERBUILD_RELAY:&str = "https://rpc.beaverbuild.org/";
pub const TITAN_RELAY:      &str = "https://rpc.titanbuilder.xyz/";
pub const ALUMNI_RELAY:     &str = "https://base.flashbots.net"; // Flashbots Base endpoint

pub static PRIVATE_RELAYS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![ALUMNI_RELAY, BEAVERBUILD_RELAY, TITAN_RELAY, PENGUIN_RELAY, RSYNC_RELAY]
});

pub const PENGUIN_RELAY:    &str = "https://rpc.penguinbuild.org";
pub const RSYNC_RELAY:      &str = "https://rsync-builder.xyz";

pub const HOT_MEMORY_BLOCKS: usize = 100;
pub const POOL_STATE_TTL_MS: u64 = 12000;
pub const MAX_FAILED_TRADES_STORED: usize = 1000;

pub const MINIMAL_PROXY_FACTORY: Address = addr!("4e59b44847b379578588920cA78FbF26c0B4956C");
pub const GHOST_SALT: &[u8] = b"ghost_protocol_v1";

pub static KNOWN_COMPETITORS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(addr!("000000000000084e0aD24b420F9bDb26b6fB0D0F"));
    set
});
pub const COMPETITOR_TIP_TOLERANCE_WEI: u128 = 1_000_000_000_000;

pub const BIDDING_TIERS: [(u128, u64); 5] = [
    (0, 5),
    (1_000_000_000_000_000, 10),
    (10_000_000_000_000_000, 20),
    (100_000_000_000_000_000, 40),
    (1_000_000_000_000_000_000, 60),
];
pub const MIN_BUILDER_TIP_WEI: u128 = 1_000_000_000_000;
pub const MAX_BRIBE_PCT: u32 = 99;
pub const MAX_BUILDER_TIP_WEI: u128 = 10_000_000_000_000_000;

pub const DUST_THRESHOLD_WEI: u128 = 100_000_000_000_000;
pub const DUST_CONVERSION_MAX_GAS_PRICE_GWEI: u64 = 30;
pub const MAX_SWEEP_GAS_PRICE_WEI: u128 = 50_000_000; // 0.05 gwei
pub const GAS_BUFFER_PERCENT: u64 = 10;

pub const BRANCH_POSITIONS_TO_SIMULATE: usize = 3; // Top, Mid, Tail
pub const MAX_BRANCH_LOSS_BPS: u64 = 1000; // 10% loss allowed from top simulation in other branches

pub static HONEYPOT_BYTECODE_SIGNATURES: Lazy<Vec<Vec<u8>>> = Lazy::new(|| {
    vec![vec![0xfe], vec![0xff], vec![0xf4], vec![0xf2]]
});
pub const MAX_ALLOWED_TAX_BPS: u64 = 300;
pub const MIN_LIQUIDITY_ETH: u128 = 1_000_000_000_000_000_000;

pub const REQUIRE_SUCCESSFUL_SIMULATION: bool = true;
pub const ALPHA_SCAN_DEPTH: usize = 5; // Deep paths for complex obscure cycles

pub const OPTIMISM_GAS_ORACLE:    Address = addr!("420000000000000000000000000000000000000F");
pub const ARBITRUM_NODE_INTERFACE:Address = addr!("00000000000000000000000000000000000000C8");

pub static L1_BASE_FEE_SCALAR: Lazy<FxHashMap<Chain, u64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Base, 1360);
    m
});

pub static L2_L1_DATA_GAS_MULTIPLIER: Lazy<FxHashMap<Chain, f64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Base, 0.2);
    m
});

pub static L2_GAS_LIMIT_MULTIPLIER: Lazy<FxHashMap<Chain, f64>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Base, 1.5);
    m
});

pub const GAS_VAULT_PERCENTAGE: u64 = 5;
pub const GAS_VAULT_ADDRESS: Option<Address> = Some(addr!("54d444D3873fdFFE7016Ebb535388cEf4983705b"));

pub static GAS_FAUCETS: Lazy<FxHashMap<Chain, Vec<Address>>> = Lazy::new(|| {
    FxHashMap::default()
});
pub const BOOTSTRAP_MIN_BALANCE_WEI: u128 = 1_000_000_000_000_000;

pub const CONGESTION_TX_COUNT: usize = 5000;
pub const MIN_INCLUSION_PROBABILITY: u8 = 100;
pub const MAX_BUILDER_LATENCY_MS: u64 = 100;
pub const MAX_STALE_BLOCKS: u64 = 1;
pub const MAX_NODE_LAG_SECONDS: u64 = 10;

pub const MAX_POOL_CACHE_SIZE: usize = 5_000;
pub const MAX_TOKEN_CACHE_SIZE: usize = 1_000;

pub static GLOBAL_PAUSE: Lazy<std::sync::atomic::AtomicBool> =
    Lazy::new(|| std::sync::atomic::AtomicBool::new(false));
pub const WATCH_ONLY_MODE: bool = false;

pub const MAX_WASH_TRADE_RATIO: f64 = 0.3;
pub const MIN_UNIQUE_TRADERS: usize = 5;

pub static MALICIOUS_OPCODES: Lazy<FxHashSet<u8>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(0xff); set.insert(0xf4); set.insert(0xf2); set.insert(0xfe);
    set
});

pub const MICRO_PROFIT_THRESHOLD_WEI: u128 = 10_000_000_000_000;
pub const MICRO_PROFIT_THRESHOLD_USD_CENTS: u64 = 2;

pub static KNOWN_FACTORY_DEPLOYERS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(addr!("1F98431c8aD98523631AE4a59f267346ea31F984"));
    set.insert(addr!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"));
    set
});
pub const FACTORY_SCAN_INTERVAL_BLOCKS: u64 = 1000;

pub static TOKEN_DECIMALS: Lazy<FxHashMap<Address, u8>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(addr!("4200000000000000000000000000000000000006"), 18u8);
    m.insert(addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"), 6u8);
    m
});

pub const MIN_PROFIT_WEI: u128 = 1;
pub const MIN_PROFIT_BPS: u64 = 5;
pub const DEFAULT_SLIPPAGE_BPS: u64 = 30;
pub const GAS_LIMIT_MULTIPLIER: f64 = 1.15;
pub const PRIORITY_FEE_MULTIPLIER: f64 = 1.2;
pub const MAX_GAS_PRICE_GWEI: u64 = 100;
pub const MAX_TOTAL_TX_FEE_WEI: u128 = 5_000_000_000_000_000; // 0.005 ETH for Base spikes
pub const STRIKE_GAS_LIMIT: u64 = 150_000;
pub const MIN_SEARCHER_BALANCE_WEI: u128 = 500_000_000_000_000; // ₹100 Safety Buffer
pub const SURVIVAL_PROFIT_MULTIPLIER: u64 = 2;

pub const UNISWAP_V3_FEE_TIERS: [u32; 4] = [100, 500, 3000, 10000];

pub const UNISWAP_V2_INIT_CODE_HASH: &str =
    "0x96e8ac4277198ff8b6f785478aa9a39f403cb768dd02cbee326c3e7da348845f";
pub const UNISWAP_V3_INIT_CODE_HASH: &str =
    "0xe34f199b19b2b4f47f68442619d555527d244f78a3297ea89325f843f87b8b54";
pub const SUSHISWAP_INIT_CODE_HASH: &str =
    "0xe18a34eb0e04b04f7a0ac29a6e80748dca96319b42c54d679cb821dca90c6303";

pub const UNISWAP_V2_ROUTER:   Address = addr!("7a250d5630B4cF539739dF2C5dAcb4c659F2488D");
pub const UNISWAP_V2_FACTORY:  Address = addr!("5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f");
pub const UNISWAP_V3_ROUTER:   Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const UNISWAP_V3_QUOTER:   Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const SUSHISWAP_ROUTER:    Address = addr!("d9e1cE17f2641f24aE83637ab66a2cca9C378B9F");
pub const SUSHISWAP_FACTORY:   Address = addr!("C0AEe478e3658e2610c5F7A4A2E1777cE9e4f2Ac");

pub const BASE_UNISWAP_V3_ROUTER:   Address = addr!("68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
pub const BASE_UNISWAP_V3_QUOTER:   Address = addr!("b27308f9F90D607463bb33eA1BeBb41C27CE5AB6");
pub const BASE_AERODROME_ROUTER:    Address = addr!("cF77a3Ba9A5DA3999247B82c4B9D43c1A7fD7c8a");
pub const BASE_AERODROME_FACTORY:   Address = addr!("420DD381b31aEf6683db6B902084c0BAc2D1b5d5");
pub const BASE_SUSHISWAP_ROUTER:    Address = addr!("6BD61e38Ecca1E08830E3fBC00FA91194863991a");
pub const BASE_SUSHISWAP_FACTORY:   Address = addr!("71524B9573c8d199127E09e2108269128355eeA0");
pub const BASE_PANCAKESWAP_FACTORY: Address = addr!("0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865");
pub const BASE_PANCAKESWAP_ROUTER:  Address = addr!("1b813dFA2931441bcAd067000E08271167739502");
pub const BASE_BASESWAP_FACTORY:    Address = addr!("FDa619b6d20975be8074d3315450bbBA58456B12");
pub const BASE_BASESWAP_ROUTER:     Address = addr!("327Df1E4de51d9752a3e019c42e5f9BB762ed144");
pub const BASE_MAVERICK_FACTORY:    Address = addr!("3708D64936496924f2b1853B0287515089304386");
pub const BASE_MAVERICK_ROUTER:     Address = addr!("792376e191C802a466D57b545f8f85f340807551");
pub const BASE_UNISWAP_V3_ROUTER_SR2: Address = addr!("262136065839E9905E990867056E137f866A4839");

// Alpha Hunter: Additional Niche Factories (Low Competition)
pub const BASE_SWAPBASED_FACTORY:   Address = addr!("04C9F118A4864700721A163744021d21DB27c11f");
pub const BASE_ALIENBASE_FACTORY:   Address = addr!("3D2d7681335A74Be482D207137f814bA688849E8");

pub const TARGET_ROUTERS: [Address; 4] = [
    BASE_AERODROME_ROUTER,
    BASE_UNISWAP_V3_ROUTER,
    BASE_UNISWAP_V3_ROUTER_SR2,
    MOONWELL_COMPTROLLER,
];

#[derive(Debug, Clone)]
pub struct DexContracts {
    pub router: Address,
    pub factory: Address,
    pub quoter: Option<Address>,
}

pub static DEX_CONTRACTS: Lazy<FxHashMap<(Chain, DexName), DexContracts>> = Lazy::new(|| {
    FxHashMap::default()
});

pub static SAFE_TOKENS: Lazy<FxHashMap<Chain, Vec<Address>>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Base, vec![
        addr!("4200000000000000000000000000000000000006"),
        addr!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
        addr!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb"),
        addr!("940181a94A35A4569E4529A3CDfB74e38FD98631"),
        addr!("4ed4E862860beD51a9570b96d89aF5E1B0Efefed"),
    ]);
    m
});

pub static CORE_TOKENS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(TOKEN_WETH);
    set.insert(TOKEN_USDC);
    set.insert(TOKEN_DAI);
    set
});

pub static CORE_POOLS: Lazy<FxHashSet<Address>> = Lazy::new(|| {
    let mut set = FxHashSet::default();
    set.insert(POOL_UNIV3_WETH_USDC_005);
    set.insert(POOL_UNIV3_WETH_USDC_030);
    set.insert(POOL_BASESWAP_WETH_USDC);
    set.insert(POOL_SUSHI_WETH_USDC);
    set
});

pub static BLACKLISTED_TOKENS: Lazy<FxHashMap<Chain, FxHashSet<Address>>> = Lazy::new(|| {
    let mut m = FxHashMap::default();
    m.insert(Chain::Base, FxHashSet::default());
    m
});

pub const MAX_VOLATILITY_THRESHOLD_BPS: u64 = 500;
pub const NETWORK_CONGESTION_GWEI: u64 = 300;
pub const GLOBAL_STOP_LOSS_BPS: u64 = 1000;
pub const BASE_BUNDLE_GAS: u64 = 800_000;
pub const MAX_BUNDLE_GAS: u64 = 10_000_000;
