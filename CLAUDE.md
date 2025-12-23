# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a MultiversX DEX Aggregator smart contract written in Rust using the `multiversx-sc` framework (v0.64.0). It executes swap paths from an external aggregator algorithm, supporting token swaps, LP minting/burning, and liquid staking across multiple DEXes.

## Build Commands

```bash
# Build the smart contract (generates WASM)
cd meta && cargo run build

# Build with release optimizations
cd meta && cargo run build --release

# Check for compilation errors
cargo check

# Run clippy lints
cargo clippy

# Run tests
cargo test

# Generate ABI
cd meta && cargo run abi
```

## Architecture

### Core Contract (`src/aggregator.rs`)
The main contract trait `Aggregator` implements a single endpoint `xo` (aggregate) that:
1. Accepts a payment and a list of `Instruction`s
2. Executes each instruction sequentially, dispatching to appropriate DEX proxies
3. Validates minimum output amount (slippage protection)
4. Returns all vault contents to the caller

### Vault System (`src/vault.rs`)
In-memory token balance tracker using `ManagedMapEncoded` for O(1) access. Manages intermediate balances during multi-hop swaps with:
- `AmountMode::Fixed` - exact amount
- `AmountMode::Ppm` - parts per million of vault balance
- `AmountMode::All` - entire balance (avoids dust)
- `AmountMode::PrevAmount` - output from previous instruction

### Storage Module (`src/storage/mod.rs`)
Uses `storage_mapper_from_address` to read pair addresses from xExchange router and Hatom controller contracts without local storage.

### Types (`src/types.rs`)
- `ActionType` enum - all supported DEX operations (xExchange, AshSwap V1/V2, OneDex, Jex, liquid staking)
- `Instruction` - atomic execution unit containing action, inputs, and optional pool address

## Supported Protocols

- **xExchange** - CPMM swaps, add/remove liquidity
- **AshSwap V1** - Curve-style StableSwap
- **AshSwap V2** - CurveCrypto pools
- **OneDex** - Multi-token path swaps
- **Jex** - CPMM and stable pools
- **EGLD Wrapping** - wrap/unwrap WEGLD
- **Liquid Staking** - Xoxno (xEGLD, LXOXNO), Hatom (sEGLD)
- **Hatom Lending** - supply/redeem hTokens

## Project Structure

```
├── src/
│   ├── aggregator.rs   # Main contract + swap_proxy module with all DEX interfaces
│   ├── storage/mod.rs  # External storage mappers (xExchange, Hatom)
│   ├── types.rs        # ActionType, AmountMode, Instruction, InputArg
│   └── vault.rs        # In-memory balance tracking
├── meta/               # Build tooling (multiversx-sc-meta-lib)
└── wasm/               # WASM output (auto-generated)
```

## Key Constants

Hardcoded contract addresses in `src/aggregator.rs`:
- `WRAPPER_SC` - WEGLD wrapper
- `ONE_DEX_ROUTER` - OneDex router
- `HATOM_STAKING`, `XEGLD_STAKING`, `LXOXNO_STAKING` - Liquid staking contracts
- `XEXCHANGE_ROUTER`, `HATOM_CONTROLLER` in storage module
