// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

interface IBalancerVault {
    function flashLoan(address recipient, address[] memory tokens, uint256[] memory amounts, bytes memory userData) external;
}

contract SovereignShadowExecutor {
    address public immutable owner;
    address public aavePool;
    address public balancerVault;
    
    address private constant UNI_V2_ROUTER = 0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24; 
    address private constant UNI_V3_ROUTER = 0x2626664c2603381E5c88859d103939a4CF97a299; 

    constructor(address _aavePool, address _balancerVault) {
        owner = msg.sender;
        aavePool = _aavePool;
        balancerVault = _balancerVault;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "!O");
        _;
    }

    function executeArbitrage(
        address loanToken,
        uint256 loanAmount,
        bytes calldata pathData
    ) external onlyOwner {
        // Fix: Correct array initialization with size (1)
        address[] memory tokens = new address[](1);
        tokens[0] = loanToken;
        uint256[] memory amounts = new uint256[](1);
        amounts[0] = loanAmount;

        IBalancerVault(balancerVault).flashLoan(address(this), tokens, amounts, pathData);
    }

    function receiveFlashLoan(
        address[] memory tokens,
        uint256[] memory amounts,
        uint256[] memory /* feeAmounts */,
        bytes memory userData
    ) external {
        require(msg.sender == balancerVault, "!V");

        (bytes memory packedPath, uint256 minProfit) = abi.decode(userData, (bytes, uint256));
        uint256 startBalance = IERC20(tokens[0]).balanceOf(address(this));

        _performSwaps(packedPath);

        uint256 endBalance = IERC20(tokens[0]).balanceOf(address(this));
        require(endBalance >= startBalance + minProfit, "NP");

        require(IERC20(tokens[0]).transfer(balancerVault, amounts[0]), "TF");
        require(IERC20(tokens[0]).transfer(owner, IERC20(tokens[0]).balanceOf(address(this))), "TF");
    }

    function withdraw(address token) external onlyOwner {
        uint256 balance = IERC20(token).balanceOf(address(this));
        require(IERC20(token).transfer(msg.sender, balance), "TF");
    }

    function withdrawETH() external onlyOwner {
        (bool success, ) = owner.call{value: address(this).balance}("");
        require(success, "ETH transfer failed");
    }

    function _performSwaps(bytes memory packedPath) internal {
        uint8 hopCount;
        // hopCount is the first byte of the data area (after 32-byte length field)
        assembly {
            hopCount := byte(0, mload(add(packedPath, 32)))
        }

        for (uint256 i = 0; i < hopCount; i++) {
            address pool;
            address tIn;
            address tOut;
            uint8 dexType;
            uint24 fee;

            // Surgical Assembly Fix: Start at add(packedPath, 33) 
            // Masking with shr(96, ...) to ensure clean 20-byte addresses
            assembly {
                let pos := add(add(packedPath, 33), mul(i, 64))
                pool := shr(96, mload(pos))
                tIn := shr(96, mload(add(pos, 20)))
                tOut := shr(96, mload(add(pos, 40)))
                dexType := byte(0, mload(add(pos, 60)))
                fee := shr(232, mload(add(pos, 61)))
            }
            
            uint256 amountIn = IERC20(tIn).balanceOf(address(this));
            
            if (dexType == 0) { // V2
                IERC20(tIn).approve(UNI_V2_ROUTER, amountIn);
                address[] memory path = new address[](2);
                path[0] = tIn;
                path[1] = tOut;
                
                (bool success, ) = UNI_V2_ROUTER.call(
                    abi.encodeWithSignature(
                        "swapExactTokensForTokens(uint256,uint256,address[],address,uint256)",
                        amountIn, 0, path, address(this), block.timestamp
                    )
                );
                require(success, "V2F");
            } else { // V3
                IERC20(tIn).approve(UNI_V3_ROUTER, amountIn);
                bytes memory params = abi.encode(tIn, tOut, fee, address(this), amountIn, 0, 0);
                (bool success, ) = UNI_V3_ROUTER.call(
                    abi.encodeWithSignature("exactInputSingle((address,address,uint24,address,uint256,uint256,uint160))", params)
                );
                require(success, "V3F");
            }
        }
    }
}