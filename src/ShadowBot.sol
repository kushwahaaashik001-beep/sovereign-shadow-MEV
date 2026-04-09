// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title ShadowBot: The Sovereign Shadow Executor
 * @dev Optimized for Base Mainnet Arbitrage using Balancer Flash Loans.
 * Efficiency: Maximum | Risk: Zero | Speed: Sub-millisecond execution logic.
 */

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address recipient, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

interface IVault {
    function flashLoan(
        address recipient,
        address[] memory tokens,
        uint256[] memory amounts,
        bytes memory userData
    ) external;
}

contract ShadowBot {
    address public immutable owner;
    
    // Pillar P: Hardcoded Cold Vault for Maximum Security & Zero-Gas lookup
    address public constant VAULT = 0x54d444D3873fdFFE7016Ebb535388cEf4983705b;
    
    IVault private constant vault = IVault(0xBA12222222228d8Ba445958a75a0704d566BF2C8);
    address private constant WETH = 0x4200000000000000000000000000000000000006;

    struct SwapStep {
        address target;   // Pool/Router address
        bytes callData;   // Encoded swap function
        address inputToken;
        address outputToken;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "SHADOW: NOT_AUTHORIZED");
        _;
    }

    constructor() {
        owner = msg.sender;
    }

    /**
     * @notice The Trigger - Rust engine calls this to start the hunt.
     */
    function ignite(
        uint256 loanAmount,
        bytes calldata encodedSteps
    ) external onlyOwner {
        address[] memory tokens = new address[](1);
        tokens[0] = WETH;
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = loanAmount;

        // Fire the Flash Loan
        vault.flashLoan(address(this), tokens, amounts, encodedSteps);
    }

    /**
     * @dev Callback from Balancer Vault. 
     * Executes arbitrage steps and ensures repayment + profit.
     */
    function receiveFlashLoan(
        address[] memory /* tokens */,
        uint256[] memory amounts,
        uint256[] memory feeAmounts,
        bytes memory userData
    ) external {
        require(msg.sender == address(vault), "SHADOW: UNTRUSTED_SOURCE");

        // Decode instructions provided by the Rust Pathfinding engine
        SwapStep[] memory steps = abi.decode(userData, (SwapStep[]));

        for (uint256 i = 0; i < steps.length; i++) {
            // Approve the target to spend our current token
            uint256 currentBalance = IERC20(steps[i].inputToken).balanceOf(address(this));
            IERC20(steps[i].inputToken).approve(steps[i].target, currentBalance);

            // Execute Low-Level Call for maximum gas efficiency
            (bool success, ) = steps[i].target.call(steps[i].callData);
            require(success, "SHADOW: SWAP_FAILED");
        }

        uint256 balanceAfter = IERC20(WETH).balanceOf(address(this));
        uint256 totalRepayment = amounts[0] + feeAmounts[0];

        // PILLAR A: Zero-Loss Guarantee
        require(balanceAfter > totalRepayment, "SHADOW: NO_PROFIT_NO_TRADE");

        // Repay the loan
        IERC20(WETH).transfer(address(vault), totalRepayment);

        // PILLAR P: Move net profit to Bot Wallet (owner) for survival logic
        uint256 netProfit = IERC20(WETH).balanceOf(address(this));
        if (netProfit > 0) {
            IERC20(WETH).transfer(owner, netProfit);
        }
    }

    /**
     * @notice Pillar P: Harvest specific tokens to the Vault.
     */
    function harvest(address token) public onlyOwner {
        uint256 balance = IERC20(token).balanceOf(address(this));
        IERC20(token).transfer(VAULT, balance);
    }

    /**
     * @notice Pillar P: Automated Vault Sweep (ETH + WETH).
     * Can be called by anyone to clear contract dust, but funds ONLY go to VAULT.
     */
    function sendToVault() public {
        uint256 ethAmount = address(this).balance;
        if (ethAmount > 0) {
            (bool success, ) = VAULT.call{value: ethAmount}("");
            require(success, "SHADOW: ETH_VAULT_TRANSFER_FAILED");
        }
        
        uint256 wethBal = IERC20(WETH).balanceOf(address(this));
        if (wethBal > 0) {
            IERC20(WETH).transfer(VAULT, wethBal);
        }
    }

    /**
     * @notice Emergency shutdown for the bot.
     */
    function withdrawETH() external onlyOwner {
        payable(owner).transfer(address(this).balance);
    }

    receive() external payable {}
}