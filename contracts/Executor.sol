// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

interface IUniswapV2Pair {
    function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
    function swap(uint amount0Out, uint amount1Out, address to, bytes calldata data) external;
}

interface IUniswapV3Pool {
    function token0() external view returns (address);
    function token1() external view returns (address);
    function fee() external view returns (uint24);
}

interface IBalancerVault {
    function flashLoan(address recipient, address[] memory tokens, uint256[] memory amounts, bytes memory userData) external;
}

/**
 * @title Sovereign Shadow Executor (V5.0 - Supreme Yul)
 * @author Supreme Lead Architect
 * @notice God-level optimized arbitrage executioner.
 */
contract Executor {
    address public immutable aavePool;
    address public immutable balancerVault;
    address public immutable owner;

    // Pillar H: Anti-Drain Protection (Transient Storage Slot - EIP-1153)
    error InsufficientProfit();
    error Unauthorized();
    error InvalidCallback();

    constructor(address _aavePool, address _balancerVault) payable {
        aavePool = _aavePool;
        balancerVault = _balancerVault;
        owner = msg.sender;
    }

    receive() external payable {}

    /**
     * @notice Arbitrage Entry Point.
     * @return profit Net profit in loanToken units.
     */
    function executeArbitrage(
        address loanToken,
        uint256 loanAmount,
        bytes calldata pathData,
        uint256 minProfit
    ) external returns (uint256 profit) {
        if (msg.sender != owner) revert Unauthorized();

        uint256 balanceBefore = IERC20(loanToken).balanceOf(address(this));

        address[] memory tokens = new address[](1);
        tokens[0] = loanToken;
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = loanAmount;

        // Pillar G: Context packing (Tight-packing for manual assembly extraction)
        // Layout: [loanToken (20 bytes)] + [pathData (bytes)]
        bytes memory userData = abi.encodePacked(loanToken, pathData);

        // Balancer Flashloan is an external call, we keep it high-level for interface safety
        // but the callback handles the rest in Yul.
        IBalancerVault(balancerVault).flashLoan(address(this), tokens, amounts, userData);
        
        uint256 balanceAfter = IERC20(loanToken).balanceOf(address(this));
        if (balanceAfter < balanceBefore + minProfit) revert InsufficientProfit();
        
        profit = balanceAfter - balanceBefore;

        // Surgical Return: Direct stack return for REVM simulator efficiency
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, profit)
            return(ptr, 32)
        }
    }

    function withdraw(address token) external {
        if (msg.sender != owner) revert Unauthorized();
        require(IERC20(token).transfer(owner, IERC20(token).balanceOf(address(this))), "TF");
    }

    function withdrawETH() external {
        if (msg.sender != owner) revert Unauthorized();
        payable(owner).transfer(address(this).balance);
    }

    function receiveFlashLoan(
        address[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory,
        bytes memory userData
    ) external {
        // High-speed safety check
        if (msg.sender != balancerVault) revert Unauthorized();

        assembly {
            // userData layout in encodePacked: [len(32)] + [loanToken(20)] + [pathData...]
            let dataStart := add(userData, 32)
            let tokenIn := shr(96, mload(dataStart))
            
            // pathData starts 20 bytes after loanToken
            let pathDataPtr := add(dataStart, 20)
            let amountIn := mload(add(amounts, 32)) // amounts[0]

            // Transfer to first pool
            mstore(0, 0xa9059cbb00000000000000000000000000000000000000000000000000000000)
            mstore(4, shr(96, mload(add(pathDataPtr, 1)))) // firstPool
            mstore(36, amountIn)
            if iszero(call(gas(), tokenIn, 0, 0, 68, 0, 0)) { revert(0, 0) }

            let hops := byte(0, mload(pathDataPtr))
            let currentPos := add(pathDataPtr, 1)

            for { let i := 0 } lt(i, hops) { i := add(i, 1) } {
                let hopPtr := add(currentPos, mul(i, 42)) // Pillar F: 42-byte Ghost alignment
                let pool := shr(96, mload(hopPtr))
                
                // Extract dexType and zeroForOne from bytes 40 and 41
                let flagsWord := mload(add(hopPtr, 40))
                let dexType := byte(0, flagsWord)
                let zeroForOne := byte(1, flagsWord)
                
                // Set activePool for callback security
                tstore(0x619888495d951d5263988d245b60648a01525060a06084890152602060a48901, pool)

                // Recipient is the next pool, or this contract if it's the last hop
                let recipient := address()
                if lt(add(i, 1), hops) {
                    recipient := shr(96, mload(add(hopPtr, 42)))
                }

                switch dexType
                case 0 { // UniswapV2
                    // Get Reserves: selector 0x0902f1ac
                    mstore(0, 0x0902f1ac00000000000000000000000000000000000000000000000000000000)
                    let success := staticcall(gas(), pool, 0, 4, 0, 64)
                    if iszero(success) { revert(0, 0) }
                    
                    let r0 := mload(0)
                    let r1 := mload(32)
                    let reserveIn := r1
                    let reserveOut := r0
                    
                    if iszero(mul(r0, r1)) { revert(0, 0) } // Anti-dust/Empty pool check

                    if zeroForOne {
                        reserveIn := r0
                        reserveOut := r1
                    }
                    
                    // amountOut = (amountIn * 997 * reserveOut) / (reserveIn * 1000 + amountIn * 997)
                    let amountInWithFee := mul(amountIn, 997)
                    let amountOut := div(mul(amountInWithFee, reserveOut), add(mul(reserveIn, 1000), amountInWithFee))
                    
                    // Call swap(amt0, amt1, to, data)
                    let ptr := mload(0x40)
                    mstore(ptr, 0x022c0d9f00000000000000000000000000000000000000000000000000000000)
                    switch zeroForOne
                    case 1 {
                        mstore(add(ptr, 4), 0)
                        mstore(add(ptr, 36), amountOut)
                    }
                    default {
                        mstore(add(ptr, 4), amountOut)
                        mstore(add(ptr, 36), 0)
                    }
                    mstore(add(ptr, 68), recipient)
                    mstore(add(ptr, 100), 128)
                    mstore(add(ptr, 132), 0)
                    
                    if iszero(call(gas(), pool, 0, ptr, 164, 0, 0)) { revert(0, 0) }
                    amountIn := amountOut
                }
                default { // UniswapV3 (dexType 1)
                    // Call swap(recipient, zeroForOne, amountSpecified, sqrtPriceLimitX96, data)
                    let ptr := mload(0x40)
                    mstore(ptr, 0x128acb0800000000000000000000000000000000000000000000000000000000)
                    mstore(add(ptr, 4), recipient) 
                    mstore(add(ptr, 36), zeroForOne)
                    mstore(add(ptr, 68), amountIn) // Exact Input
                    
                    let sqrtLimit := 4295128739 // MIN_SQRT_RATIO + 1
                    if iszero(zeroForOne) {
                        sqrtLimit := 1461446703485210103287273052203988822378723970341 // MAX_SQRT_RATIO - 1
                    }
                    mstore(add(ptr, 100), sqrtLimit)
                    mstore(add(ptr, 132), 160) // Offset to data
                    mstore(add(ptr, 164), 32)  // Data length 32 (Ghost Token Pass)
                    mstore(add(ptr, 196), tokenIn)
                    
                    if iszero(call(gas(), pool, 0, ptr, 228, 0, 0)) { revert(0, 0) }
                    
                    // Update amountIn for next hop using return data (int256 amount0, int256 amount1)
                    returndatacopy(ptr, 0, 64)
                    let a0 := mload(ptr)
                    let a1 := mload(add(ptr, 32))

                    amountIn := a0
                    if slt(a1, a0) { amountIn := a1 }
                    if slt(amountIn, 0) { amountIn := sub(0, amountIn) }
                }
                tokenIn := shr(96, mload(add(hopPtr, 20)))
            }
        }

        // Repay Flash Loan
        require(IERC20(tokens[0]).transfer(balancerVault, amounts[0]), "TF");
    }

    function uniswapV3SwapCallback(int256 amount0Delta, int256 amount1Delta, bytes calldata data) external {
        assembly {
            let activePoolSlot := 0x619888495d951d5263988d245b60648a01525060a06084890152602060a48901
            // [GOD-MODE SECURITY] Verify caller is the current active pool using TLOAD (100 gas)
            let _activePool := tload(activePoolSlot)
            if iszero(eq(caller(), _activePool)) {
                mstore(0, 0x09bde339) // InvalidCallback() selector
                revert(28, 4)
            }

            // Extract payment amount from positive delta
            let amount := amount0Delta 
            if sgt(amount1Delta, 0) { amount := amount1Delta }

            // Extract tokenIn from calldata (Ghost Protocol)
            let token := calldataload(data.offset)
            mstore(0, 0xa9059cbb) // transfer(to, amount)
            mstore(32, caller())
            mstore(64, amount)
            if iszero(call(gas(), token, 0, 28, 68, 0, 0)) { revert(0, 0) }
        }
    }
}