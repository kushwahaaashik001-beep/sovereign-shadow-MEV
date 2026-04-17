// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IUniswapV2Pair {
    function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
    function swap(uint amount0Out, uint amount1Out, address to, bytes calldata data) external;
    function token0() external view returns (address);
    function token1() external view returns (address);
}

interface IUniswapV3Pool {
    function token0() external view returns (address);
    function token1() external view returns (address);
    function swap(address recipient, bool zeroForOne, int256 amountSpecified, uint160 sqrtPriceLimitX96, bytes calldata data) external returns (int256 amount0, int256 amount1);
}

interface IPool {
    function flashLoan(
        address receiverAddress,
        address[] calldata assets,
        uint256[] calldata amounts,
        uint256[] calldata interestRateModes,
        address onBehalfOf,
        bytes calldata params,
        uint16 referralCode
    ) external;
}

interface IVault {
    function flashLoan(
        address recipient,
        address[] memory tokens,
        uint256[] memory amounts,
        bytes memory userData
    ) external;
}

/**
 * @title ShadowBot
 * @dev Implements Pillar N (Zero-Loss Shield) and Pillar M (Flash Loans)
 */
contract ShadowBot {
    address public immutable owner; // Pillar G: Immutable for Zero-SLOAD execution
    address public constant BALANCER_VAULT = 0xBA12222222228d8Ba445958a75a0704d566BF2C8;
    address public constant AAVE_V3_POOL = 0xA238Dd80C259a72e81d7e4664a9801593F98d1c5;
    
    uint160 internal constant MIN_SQRT_RATIO = 4295128739;
    uint160 internal constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    // Optimized ERC20 selectors
    bytes4 private constant TRANSFER_SELECTOR = 0xa9059cbb;
    bytes4 private constant BALANCEOF_SELECTOR = 0x70a08231;

    error ZeroLossShieldActivated();
    error Unauthorized();

    /**
     * @dev Stateless execution. No persistent data storage during swaps 
     * to ensure "Ghost" behavior (Pillar G).
     */
    modifier onlyOwner() {
        if (msg.sender != owner) {
            assembly {
                mstore(0x00, 0x82b42900) // Unauthorized()
                revert(0x1c, 0x04)
            }
        }
        _;
    }

    constructor() {
        owner = msg.sender;
    }

    /**
     * @dev Bug 5 Fixed: Explicit ABI entry point for the Rust executor.
     * It receives the loan details and the ghost-encoded path data.
     */
    function executeArbitrage(
        bytes calldata pathData
    ) external onlyOwner {
        _initiateExecution(pathData);
    }

    /**
     * Entry point called by the Rust bot.
     * Expects calldata matching encode_ghost_multi in models.rs
     */
    fallback() external payable {
        if (msg.sender != owner) revert Unauthorized();
        if (msg.data.length < 34) revert("Invalid payload");
        _initiateExecution(msg.data);
    }

    receive() external payable {}

    function _initiateExecution(bytes calldata data) internal {
        uint8 lenderId;
        uint8 loansLen;
        assembly {
            let word := calldataload(add(data.offset, 32))
            lenderId := byte(0, word)
            loansLen := byte(1, word)
        }

        if (lenderId == 0) { 
            _initBalancerFlash(data, loansLen); 
        } else if (lenderId == 1) { 
            _initAaveFlash(data, loansLen);
        }
    }

    function _initBalancerFlash(bytes calldata data, uint8 loansLen) internal {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, 0xab512f51) // flashLoan(address,address[],uint256[],bytes)
            mstore(add(ptr, 0x04), address())
            mstore(add(ptr, 0x24), 0x80) // tokens offset
            let amountsOff := add(0x80, add(0x20, mul(loansLen, 0x20)))
            mstore(add(ptr, 0x44), amountsOff) // amounts offset
            let userDataOff := add(amountsOff, add(0x20, mul(loansLen, 0x20)))
            mstore(add(ptr, 0x64), userDataOff) // userData offset

            mstore(add(ptr, 0x84), loansLen)
            let dataPtr := add(data.offset, 34)
            for { let i := 0 } lt(i, loansLen) { i := add(i, 1) } {
                mstore(add(ptr, add(0xa4, mul(i, 0x20))), shr(96, calldataload(dataPtr)))
                dataPtr := add(dataPtr, 52)
            }
            mstore(add(ptr, add(amountsOff, 0x04)), loansLen)
            dataPtr := add(data.offset, 54)
            for { let i := 0 } lt(i, loansLen) { i := add(i, 1) } {
                mstore(add(ptr, add(add(amountsOff, 0x24), mul(i, 0x20))), calldataload(dataPtr))
                dataPtr := add(dataPtr, 52)
            }
            mstore(add(ptr, add(userDataOff, 0x04)), data.length)
            calldatacopy(add(ptr, add(userDataOff, 0x24)), data.offset, data.length)
            if iszero(call(gas(), 0xBA12222222228d8Ba445958a75a0704d566BF2C8, 0, ptr, add(userDataOff, add(0x24, data.length)), 0, 0)) { revert(0, 0) }
        }
    }

    function _initAaveFlash(bytes calldata data, uint8 loansLen) internal {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, 0x0b69324d) // flashLoan(address,address[],uint256[],uint256[],address,bytes,uint16)
            mstore(add(ptr, 0x04), address())
            mstore(add(ptr, 0x24), 0xc0) // assets offset
            let amountsOff := add(0xc0, add(0x20, mul(loansLen, 0x20)))
            mstore(add(ptr, 0x44), amountsOff) // amounts offset
            let modesOff := add(amountsOff, add(0x20, mul(loansLen, 0x20)))
            mstore(add(ptr, 0x64), modesOff) // modes offset
            mstore(add(ptr, 0x84), address()) // onBehalfOf
            let paramsOff := add(modesOff, add(0x20, mul(loansLen, 0x20)))
            mstore(add(ptr, 0xa4), paramsOff) // params offset
            mstore(add(ptr, 0xc4), 0) // referralCode
            mstore(add(ptr, 0xe4), loansLen)
            let dataPtr := add(data.offset, 34)
            for { let i := 0 } lt(i, loansLen) { i := add(i, 1) } {
                mstore(add(ptr, add(0x104, mul(i, 0x20))), shr(96, calldataload(dataPtr)))
                dataPtr := add(dataPtr, 52)
            }
            mstore(add(ptr, add(amountsOff, 0x04)), loansLen)
            dataPtr := add(data.offset, 54)
            for { let i := 0 } lt(i, loansLen) { i := add(i, 1) } {
                mstore(add(ptr, add(add(amountsOff, 0x24), mul(i, 0x20))), calldataload(dataPtr))
                dataPtr := add(dataPtr, 52)
            }
            mstore(add(ptr, add(modesOff, 0x04)), loansLen)
            mstore(add(ptr, add(paramsOff, 0x04)), data.length)
            calldatacopy(add(ptr, add(paramsOff, 0x24)), data.offset, data.length)
            if iszero(call(gas(), 0xA238Dd80C259a72e81d7e4664a9801593F98d1c5, 0, ptr, add(paramsOff, add(0x24, data.length)), 0, 0)) { revert(0, 0) }
        }
    }

    /**
     * Balancer V2 Callback: Here is where the arbitrage happens.
     */
    function receiveFlashLoan(
        address[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory feeAmounts,
        bytes memory userData
    ) external {
        require(msg.sender == BALANCER_VAULT, "Only Balancer Vault");

        uint256 balanceBefore;
        address primaryToken;
        uint256 minProfit;

        assembly {
            minProfit := mload(add(userData, 32))
            primaryToken := mload(add(tokens, 0x20))
            mstore(0x00, 0x70a08231)
            mstore(0x04, address())
            if iszero(staticcall(gas(), primaryToken, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            balanceBefore := sub(mload(0x20), mload(add(amounts, 0x20)))
        }

        _executeArbitrageHops(userData); 

        assembly {
            let len := mload(tokens)
            for { let i := 0 } lt(i, len) { i := add(i, 1) } {
                let token := mload(add(add(tokens, 0x20), mul(i, 0x20)))
                let total := add(mload(add(add(amounts, 0x20), mul(i, 0x20))), mload(add(add(feeAmounts, 0x20), mul(i, 0x20))))
                let ptr := mload(0x40)
                mstore(ptr, 0xa9059cbb)
                mstore(add(ptr, 0x04), 0xBA12222222228d8Ba445958a75a0704d566BF2C8)
                mstore(add(ptr, 0x24), total)
                if iszero(call(gas(), token, 0, ptr, 0x44, 0, 0)) { revert(0, 0) }
            }
            mstore(0x00, 0x70a08231)
            mstore(0x04, address())
            if iszero(staticcall(gas(), primaryToken, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            if lt(mload(0x20), add(balanceBefore, minProfit)) {
                mstore(0x00, 0x937c4424) // ZeroLossShieldActivated()
                revert(0x1c, 0x04)
            }
            if gt(selfbalance(), 0) {
                pop(call(gas(), 0x4200000000000000000000000000000000000011, selfbalance(), 0, 0, 0, 0))
            }
        }
    }

    /**
     * Aave V3 Callback: Atomic arbitrage execution.
     */
    function executeOperation(
        address[] calldata assets,
        uint256[] calldata amounts,
        uint256[] calldata premiums,
        address,
        bytes calldata params
    ) external returns (bool) {
        require(msg.sender == AAVE_V3_POOL, "Only Aave V3 Pool");

        uint256 minProfit;
        uint256 balanceBefore;
        address primaryToken = assets[0];

        assembly {
            minProfit := calldataload(params.offset)
            
            mstore(0x00, 0x70a08231) // balanceOf(address)
            mstore(0x04, address())
            if iszero(staticcall(gas(), primaryToken, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            balanceBefore := sub(mload(0x20), mload(add(amounts.offset, 0x20)))
        }

        // Pillar N: Avoid Calldata-to-Memory Copy
        _executeArbitrageHopsCalldata(params); 

        // Pillar M: Multi-Asset Repayment Loop
        for (uint256 i = 0; i < assets.length; i++) {
            _safeApprove(assets[i], AAVE_V3_POOL, amounts[i] + premiums[i]);
        }

        assembly {
            mstore(0x00, 0x70a08231)
            mstore(0x04, address())
            if iszero(staticcall(gas(), primaryToken, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            let balanceAfter := mload(0x20)
            
            if lt(balanceAfter, add(balanceBefore, minProfit)) {
                mstore(0x00, 0x937c4424) // ZeroLossShieldActivated()
                revert(0x1c, 0x04)
            }

            if gt(selfbalance(), 0) {
                let success := call(gas(), coinbase(), selfbalance(), 0, 0, 0, 0)
            }
        }

        return true;
    }

    /**
     * @dev Ultra-optimized Binary Decoder for calldata (Sword Check).
     */
    function _executeArbitrageHopsCalldata(bytes calldata data) internal {
        assembly {
            // data.offset is the starting point in calldata
            let loansLen := byte(1, calldataload(add(data.offset, 32))) 
            let hopsPtr := add(add(data.offset, 34), mul(loansLen, 52))
            let hopsLen := byte(0, calldataload(hopsPtr))
            let currentHop := add(hopsPtr, 1)

            for { let i := 0 } lt(i, hopsLen) { i := add(i, 1) } {
                let pool := shr(96, calldataload(currentHop))
                let tokenOut := shr(96, calldataload(add(currentHop, 20))) 
                let word := calldataload(add(currentHop, 40))
                let dexType := byte(0, word)
                let zfo := byte(1, word)
                let isStable := byte(2, word)

                switch dexType
                case 0 { _executeV2(pool, tokenOut, zfo) }
                case 3 { _executeAerodrome(pool, tokenOut, zfo, isStable) }
                case 1 { _executeV3(pool, zfo, tokenOut) }
                
                currentHop := add(currentHop, 43)
            }

            function _executeAerodrome(pool, tokenOut, zfo, isStable) {
                let tokenIn := _getV2TokenLogic(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                _safeTransferYul(tokenIn, pool, bal)
                
                let amountOut
                switch isStable
                case 1 { amountOut := _getAerodromeStableAmountOut(pool, bal, zfo) }
                default { amountOut := _getV2AmountOut(pool, tokenOut, zfo) }
                
                let out0 := 0 let out1 := 0
                if zfo { out1 := amountOut } { out0 := amountOut }
                let ptr := mload(0x40)
                mstore(ptr, 0x022c0d9f) // swap(...)
                mstore(add(ptr, 0x04), out0)
                mstore(add(ptr, 0x24), out1)
                mstore(add(ptr, 0x44), address())
                mstore(add(ptr, 0x64), 0x80)
                mstore(add(ptr, 0x84), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xa4, 0, 0)) { revert(0, 0) }
            }

            function _getAerodromeStableAmountOut(pool, amtIn, zfo) -> amtOut {
                // Ultra-Fast Yul implementation of Aerodrome Stable Curve x^3y + y^3x = k
                // We fetch reserves and perform 2 iterations of Newton-Raphson for nanosecond precision.
                let ptr := mload(0x40)
                mstore(ptr, 0x0902f1ac) // getReserves()
                if iszero(staticcall(gas(), pool, ptr, 0x04, ptr, 0x64)) { revert(0, 0) }
                let r0 := mload(ptr) let r1 := mload(add(ptr, 0x20))
                let x := r1 let y := r0 if zfo { x := r0 y := r1 }
                
                let k := div(add(mul(mul(mul(x, x), x), y), mul(mul(mul(y, y), y), x)), 1000000000000000000)
                let x_new := add(x, div(mul(amtIn, 9998), 10000)) // 0.02% fee assumption
                
                let y_curr := y
                let x3 := mul(mul(x_new, x_new), x_new)
                for { let j := 0 } lt(j, 2) { j := add(j, 1) } {
                    let y2 := mul(y_curr, y_curr)
                    let f_y := sub(add(div(mul(x3, y_curr), 1000000000000000000), div(mul(mul(y2, y_curr), x_new), 1000000000000000000)), k)
                    let f_prime := add(div(x3, 1000000000000000000), div(mul(mul(3, y2), x_new), 1000000000000000000))
                    y_curr := sub(y_curr, div(mul(f_y, 1000000000000000000), f_prime))
                }
                amtOut := sub(y, y_curr)
            }
            function _executeV2(pool, tokenOut, zfo) {
                let tokenIn := _getV2TokenLogic(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                if iszero(bal) { revert(0, 0) }
                _safeTransferYul(tokenIn, pool, bal)
                
                let amountOut := _getV2AmountOut(pool, tokenOut, zfo)
                let out0 := 0
                let out1 := 0
                if zfo { out1 := amountOut } { out0 := amountOut }
                let ptr := mload(0x40)
                mstore(ptr, 0x022c0d9f) // swap(uint256,uint256,address,bytes)
                mstore(add(ptr, 0x04), out0)
                mstore(add(ptr, 0x24), out1)
                mstore(add(ptr, 0x44), address())
                mstore(add(ptr, 0x64), 0x80)
                mstore(add(ptr, 0x84), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xa4, 0, 0)) { revert(0, 0) }
            }

            function _executeV3(pool, zfo, tokenOut) {
                let tokenIn := _getV3TokenIn(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                if iszero(bal) { revert(0, 0) }

                let ptr := mload(0x40)
                mstore(ptr, 0x128acb08) // swap(address,bool,int256,uint160,bytes)
                mstore(add(ptr, 0x04), address())
                mstore(add(ptr, 0x24), zfo)
                mstore(add(ptr, 0x44), sub(0, bal)) // Exact input: negative balance
                
                let sqrtLimit := 4295128740 
                if iszero(zfo) { sqrtLimit := 1461446703485210103287273052203988822378723970341 }
                
                mstore(add(ptr, 0x64), sqrtLimit)
                mstore(add(ptr, 0x84), 0xa0)
                mstore(add(ptr, 0xa4), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xc4, 0, 0)) { revert(0, 0) }
            }

            function _getV3TokenIn(pool, zfo) -> tIn {
                let ptr := mload(0x40)
                let selector := 0xd21220a7 // token1()
                if zfo { selector := 0x0dfe1681 } // token0()
                mstore(ptr, selector)
                if iszero(staticcall(gas(), pool, ptr, 0x04, ptr, 0x20)) { revert(0, 0) }
                tIn := mload(ptr)
            }

            function _getV2TokenLogic(pool, zfo) -> tIn {
                let ptr := mload(0x40)
                let selector := 0x0dfe1681 // token0
                if iszero(zfo) { selector := 0xd21220a7 } // if 1->0, in is token1
                mstore(ptr, selector)
                if iszero(staticcall(gas(), pool, ptr, 0x04, ptr, 0x20)) { revert(0, 0) }
                tIn := mload(ptr)
            }

            function _getV2AmountOut(_pool, _tokenOut, _zfo) -> amountOut {
                let ptr := mload(0x40)
                mstore(ptr, 0x0902f1ac) // getReserves()
                if iszero(staticcall(gas(), _pool, ptr, 0x04, ptr, 0x64)) { revert(0, 0) }
                let r0 := mload(ptr)
                let r1 := mload(add(ptr, 0x20))
                let resIn := r1 let resOut := r0
                if _zfo { resIn := r0 resOut := r1 }
                let tokenIn := _getV2TokenLogic(_pool, _zfo)
                let amountIn := sub(_getContractBalance_for_pool(tokenIn, _pool), resIn)
                let amountInWithFee := mul(amountIn, 997)
                amountOut := div(mul(amountInWithFee, resOut), add(mul(resIn, 1000), amountInWithFee))
            }

            function _getContractBalance(_token) -> bal {
                let ptr := mload(0x40)
                mstore(ptr, 0x70a08231)
                mstore(add(ptr, 0x04), address())
                if iszero(staticcall(gas(), _token, ptr, 0x24, ptr, 0x20)) { revert(0, 0) }
                bal := mload(ptr)
            }

            function _getContractBalance_for_pool(_token, _pool) -> bal {
                let ptr := mload(0x40)
                mstore(ptr, 0x70a08231)
                mstore(add(ptr, 0x04), _pool)
                if iszero(staticcall(gas(), _token, ptr, 0x24, ptr, 0x20)) { revert(0, 0) }
                bal := mload(ptr)
            }

            function _safeTransferYul(token, to, value) {
                let ptr := mload(0x40)
                mstore(ptr, 0xa9059cbb)
                mstore(add(ptr, 0x04), to)
                mstore(add(ptr, 0x24), value)
                if iszero(call(gas(), token, 0, ptr, 0x44, 0, 0)) { revert(0, 0) }
            }
        }
    }

    /**
     * @dev Ultra-optimized Binary Decoder for memory payload (Balancer Callback).
     * Layout: [32b minProfit][1b lenderId][1b loansLen][N*52b loans][1b hopsLen][M*42b hops]
     */
    function _executeArbitrageHops(bytes memory data) internal {
        assembly {
            // Memory layout: [len 32][minProfit 32][lenderId 1][loansLen 1][loans N*52][hopsLen 1][hops M*42]
            let content := add(data, 0x20)
            let loansLen := byte(1, mload(add(content, 32))) 
            let hopsPtr := add(add(content, 34), mul(loansLen, 52))
            let hopsLen := byte(0, mload(hopsPtr))
            let currentHop := add(hopsPtr, 1)

            for { let i := 0 } lt(i, hopsLen) { i := add(i, 1) } {
                let pool := shr(96, mload(currentHop))
                let tokenOut := shr(96, mload(add(currentHop, 20))) 
                let word := mload(add(currentHop, 40))
                let dexType := byte(0, word)
                let zfo := byte(1, word)
                let isStable := byte(2, word)

                switch dexType
                case 0 { _executeV2_Mem(pool, tokenOut, zfo) }
                case 3 { _executeAerodrome_Mem(pool, tokenOut, zfo, isStable) }
                case 1 { _executeV3_Mem(pool, zfo, tokenOut) }
                
                // Bug 4 Fixed: Hop size is 43 bytes (20+20+1+1+1)
                currentHop := add(currentHop, 43) 
            }

            function _executeAerodrome_Mem(pool, tokenOut, zfo, isStable) {
                let tokenIn := _getV2TokenLogic(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                _safeTransferYul(tokenIn, pool, bal)
                
                let amountOut
                switch isStable
                case 1 { amountOut := _getAerodromeStableAmountOut(pool, bal, zfo) }
                default { amountOut := _getV2AmountOut(pool, tokenOut, zfo) }
                
                let out0 := 0 let out1 := 0
                if zfo { out1 := amountOut } { out0 := amountOut }
                let ptr := mload(0x40)
                mstore(ptr, 0x022c0d9f)
                mstore(add(ptr, 0x04), out0)
                mstore(add(ptr, 0x24), out1)
                mstore(add(ptr, 0x44), address())
                mstore(add(ptr, 0x64), 0x80)
                mstore(add(ptr, 0x84), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xa4, 0, 0)) { revert(0, 0) }
            }

            function _executeV2_Mem(pool, tokenOut, zfo) {
                let tokenIn := _getV2TokenLogic(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                if iszero(bal) { revert(0, 0) }
                _safeTransferYul(tokenIn, pool, bal)
                
                let amountOut := _getV2AmountOut(pool, tokenOut, zfo)
                let out0 := 0
                let out1 := 0
                if zfo { out1 := amountOut } { out0 := amountOut }
                let ptr := mload(0x40)
                mstore(ptr, 0x022c0d9f)
                mstore(add(ptr, 0x04), out0)
                mstore(add(ptr, 0x24), out1)
                mstore(add(ptr, 0x44), address())
                mstore(add(ptr, 0x64), 0x80)
                mstore(add(ptr, 0x84), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xa4, 0, 0)) { revert(0, 0) }
            }

            function _executeV3_Mem(pool, zfo, tokenOut) {
                let tokenIn := _getV3TokenIn(pool, zfo)
                let bal := _getContractBalance(tokenIn)
                if iszero(bal) { revert(0, 0) }
                let ptr := mload(0x40)
                mstore(ptr, 0x128acb08)
                mstore(add(ptr, 0x04), address())
                mstore(add(ptr, 0x24), zfo)
                mstore(add(ptr, 0x44), sub(0, bal))
                let sqrtLimit := 4295128740 
                if iszero(zfo) { sqrtLimit := 1461446703485210103287273052203988822378723970341 }
                mstore(add(ptr, 0x64), sqrtLimit)
                mstore(add(ptr, 0x84), 0xa0)
                mstore(add(ptr, 0xa4), 0)
                if iszero(call(gas(), pool, 0, ptr, 0xc4, 0, 0)) { revert(0, 0) }
            }

            function _getV3TokenIn(pool, zfo) -> tIn {
                let ptr := mload(0x40)
                let selector := 0xd21220a7 // token1()
                if zfo { selector := 0x0dfe1681 } // token0()
                mstore(ptr, selector)
                if iszero(staticcall(gas(), pool, ptr, 0x04, ptr, 0x20)) { revert(0, 0) }
                tIn := mload(ptr)
            }

            function _getV2TokenLogic(pool, zfo) -> tIn {
                let ptr := mload(0x40)
                let selector := 0x0dfe1681 // token0
                if iszero(zfo) { selector := 0xd21220a7 } // if 1->0, in is token1
                mstore(ptr, selector)
                if iszero(staticcall(gas(), pool, ptr, 0x04, ptr, 0x20)) { revert(0, 0) }
                tIn := mload(ptr)
            }

            function _getV2AmountOut(_pool, _tokenOut, _zfo) -> amountOut {
                let ptr := mload(0x40)
                mstore(ptr, 0x0902f1ac) // getReserves()
                if iszero(staticcall(gas(), _pool, ptr, 0x04, ptr, 0x64)) { revert(0, 0) }
                let r0 := mload(ptr)
                let r1 := mload(add(ptr, 0x20))
                let resIn := r1 let resOut := r0
                if _zfo { resIn := r0 resOut := r1 }
                let tokenIn := _getV2TokenLogic(_pool, _zfo)
                let amountIn := sub(_getContractBalance_for_pool(tokenIn, _pool), resIn)
                let amountInWithFee := mul(amountIn, 997)
                amountOut := div(mul(amountInWithFee, resOut), add(mul(resIn, 1000), amountInWithFee))
            }

            function _getContractBalance(_token) -> bal {
                let ptr := mload(0x40)
                mstore(ptr, 0x70a08231)
                mstore(add(ptr, 0x04), address())
                if iszero(staticcall(gas(), _token, ptr, 0x24, ptr, 0x20)) { revert(0, 0) }
                bal := mload(ptr)
            }

            function _getContractBalance_for_pool(_token, _pool) -> bal {
                let ptr := mload(0x40)
                mstore(ptr, 0x70a08231)
                mstore(add(ptr, 0x04), _pool)
                if iszero(staticcall(gas(), _token, ptr, 0x24, ptr, 0x20)) { revert(0, 0) }
                bal := mload(ptr)
            }

            function _safeTransferYul(token, to, value) {
                let ptr := mload(0x40)
                mstore(ptr, 0xa9059cbb)
                mstore(add(ptr, 0x04), to)
                mstore(add(ptr, 0x24), value)
                if iszero(call(gas(), token, 0, ptr, 0x44, 0, 0)) { revert(0, 0) }
            }
        }
    }

    function _swapV2(address pool, bool zeroForOne) internal {
        IUniswapV2Pair pair = IUniswapV2Pair(pool);
        (uint256 r0, uint256 r1, ) = pair.getReserves();
        
        address tokenIn = zeroForOne ? pair.token0() : pair.token1();
        uint256 amountIn;
        assembly {
            mstore(0x00, 0x70a08231)
            mstore(0x04, address())
            if iszero(staticcall(gas(), tokenIn, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            amountIn := mload(0x20)
        }

        _safeTransfer(tokenIn, pool, amountIn);

        uint256 amountInWithFee = amountIn * 997;
        if (zeroForOne) {
            uint256 amountOut = (amountInWithFee * r1) / (r0 * 1000 + amountInWithFee);
            pair.swap(0, amountOut, address(this), "");
        } else {
            uint256 amountOut = (amountInWithFee * r0) / (r1 * 1000 + amountInWithFee);
            pair.swap(amountOut, 0, address(this), "");
        }
    }

    function _swapV3(address pool, bool zeroForOne) internal {
        IUniswapV3Pool v3Pool = IUniswapV3Pool(pool);
        address tokenIn = zeroForOne ? v3Pool.token0() : v3Pool.token1();
        uint256 amountIn;
        assembly {
            mstore(0x00, 0x70a08231)
            mstore(0x04, address())
            if iszero(staticcall(gas(), tokenIn, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            amountIn := mload(0x20)
        }

        v3Pool.swap(
            address(this),
            zeroForOne,
            int256(amountIn),
            zeroForOne ? MIN_SQRT_RATIO + 1 : MAX_SQRT_RATIO - 1,
            ""
        );
    }

    function uniswapV3SwapCallback(int256 amount0Delta, int256 amount1Delta, bytes calldata) external {
        if (amount0Delta > 0) _safeTransfer(IUniswapV3Pool(msg.sender).token0(), msg.sender, uint256(amount0Delta));
        else if (amount1Delta > 0) _safeTransfer(IUniswapV3Pool(msg.sender).token1(), msg.sender, uint256(amount1Delta));
    }

    function _safeTransfer(address token, address to, uint256 value) internal {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, 0xa9059cbb)
            mstore(add(ptr, 0x04), to)
            mstore(add(ptr, 0x24), value)
            let success := call(gas(), token, 0, ptr, 0x44, ptr, 0x20)
            if success {
                if gt(returndatasize(), 0) { success := mload(ptr) }
            }
            if iszero(success) { revert(0, 0) }
        }
    }

    function _safeApprove(address token, address spender, uint256 value) internal {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, 0x095ea7b3)
            mstore(add(ptr, 0x04), spender)
            
            // USDT Fix: Set to 0 first
            mstore(add(ptr, 0x24), 0)
            pop(call(gas(), token, 0, ptr, 0x44, 0, 0))
            
            mstore(add(ptr, 0x24), value)
            let success := call(gas(), token, 0, ptr, 0x44, ptr, 0x20)
            if success {
                if gt(returndatasize(), 0) { success := mload(ptr) }
            }
            if iszero(success) { revert(0, 0) }
        }
    }

    function withdrawETH() external onlyOwner {
        assembly {
            let success := call(gas(), caller(), selfbalance(), 0, 0, 0, 0)
        }
    }

    function withdrawToken(address token) external onlyOwner {
        // Pillar J: Real implementation for token sweeping
        uint256 bal;
        assembly {
            mstore(0x00, 0x70a08231) // balanceOf(address)
            mstore(0x04, address())
            if iszero(staticcall(gas(), token, 0x00, 0x24, 0x20, 0x20)) { revert(0, 0) }
            bal := mload(0x20)
        }
        _safeTransfer(token, owner, bal);
    }
}