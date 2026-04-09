use ethers::prelude::*;

abigen!(
    IUniswapV2Factory,
    "src/abi/uniswap_v2_factory.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    IUniswapV2Pair,
    "src/abi/uniswap_v2_pair.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    IAaveV3Pool,
    "src/abi/aave_v3_pool.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    IBalancerV2Vault,
    "src/abi/balancer_v2_vault.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    IUniswapV3Factory,
    "src/abi/uniswap_v3_factory.json",
    event_derives(serde::Deserialize, serde::Serialize)
);

abigen!(
    IUniswapV3Pool,
    "src/abi/uniswap_v3_pool.json",
    event_derives(serde::Deserialize, serde::Serialize)
);
