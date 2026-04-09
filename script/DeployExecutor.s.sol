// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Script.sol";
import "../contracts/Executor.sol";

contract DeployExecutor is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("SHADOW_PRIVATE_KEY");
        address owner = vm.addr(deployerPrivateKey);

        vm.startBroadcast(deployerPrivateKey);

        // Base Mainnet Constants for Zero-Budget Flash Loans
        address aavePool = 0xA238Dd80C259a72e81d7e4664a9801593F98d1c5;
        address balancerVault = 0xBA12222222228d8Ba445958a75a0704d566BF2C8;

        // Deploying the Sovereign Shadow Muscle
        Executor executor = new Executor(aavePool, balancerVault);

        console.log("--------------------------------------------------");
        console.log(unicode"🚀 Executor Deployed Successfully!");
        console.log(unicode"📍 Address:", address(executor));
        console.log(unicode"👤 Owner:", owner);
        console.log("--------------------------------------------------");

        vm.stopBroadcast();
    }
}