use multiversx_sc::derive_imports::*;
multiversx_sc::imports!();

/// Supported DEX types - the contract knows how to call each one
#[type_abi]
#[derive(
    TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, PartialEq, Debug, ManagedVecItem,
)]
pub enum ActionType<M: ManagedTypeApi> {
    // xExchange operations (CPMM)
    XExchangeSwap(TokenIdentifier<M>), // Output token identifier
    XExchangeAddLiquidity,
    XExchangeRemoveLiquidity,

    // AshSwap V1 Stable (Curve-style StableSwap)
    AshSwapPoolSwap(TokenIdentifier<M>), // Output token identifier
    AshSwapPoolAddLiquidity,
    AshSwapPoolRemoveLiquidity(u32), // Count of output tokens

    // AshSwap V2 (CurveCrypto)
    AshSwapV2Swap,
    AshSwapV2AddLiquidity,
    AshSwapV2RemoveLiquidity(u32), // Count of output tokens

    // OneDex operations
    OneDexSwap(TokenIdentifier<M>), // Output token identifier
    OneDexAddLiquidity(usize),      // Pair ID
    OneDexRemoveLiquidity,

    // Jex CPMM operations
    JexSwap,
    JexAddLiquidity,
    JexRemoveLiquidity,

    // Jex Stable operations
    JexStableSwap(TokenIdentifier<M>), // Output token identifier
    JexStableAddLiquidity,
    JexStableRemoveLiquidity,

    // EGLD wrapping
    Wrapping,
    UnWrapping,

    // Liquid staking
    XoxnoLiquidStaking,
    LXoxnoLiquidStaking,
    HatomLiquidStaking,

    // Hatom operations
    HatomRedeem,
    HatomSupply(TokenIdentifier<M>), // hToken identifier output token
}

/// How to determine the input amount for an instruction
#[type_abi]
#[derive(
    TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, PartialEq, Debug, ManagedVecItem,
)]
pub enum AmountMode<M: ManagedTypeApi> {
    /// Fixed amount specified (first hop when input amount is known).
    Fixed(BigUint<M>),
    /// Parts per million of vault balance.
    /// 1_000_000 = 100%, 600_000 = 60%.
    Ppm(u32),
    /// Use entire vault balance of the token.
    /// Used for the last instruction touching a token to avoid dust.
    All,
    /// Use the output from the previous instruction as input.
    /// This creates a chain within a path where tokens flow directly
    /// without touching the shared vault, preventing conflicts with
    /// other tokens that may share the same intermediate token.
    PrevAmount,
}

/// Input argument for an instruction
#[type_abi]
#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, ManagedVecItem)]
pub struct InputArg<M: ManagedTypeApi> {
    pub token: ManagedBuffer<M>,
    pub mode: AmountMode<M>,
}

/// The atomic unit of execution in the aggregator
#[type_abi]
#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, ManagedVecItem)]
pub struct Instruction<M: ManagedTypeApi> {
    /// Which DEX operation to perform
    pub action: ActionType<M>,
    /// List of input assets and amounts
    pub inputs: Option<ManagedVec<M, InputArg<M>>>,
    /// Pool contract address
    pub address: Option<ManagedAddress<M>>,
}

// External

#[type_abi]
#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, PartialEq)]
pub struct PairTokens<M: ManagedTypeApi> {
    pub first_token_id: TokenIdentifier<M>,
    pub second_token_id: TokenIdentifier<M>,
}

#[type_abi]
#[derive(TopEncode, TopDecode, Copy, Clone, PartialEq, Debug)]
pub enum PairFee {
    Percent04,
    Percent06,
    Percent10,
}

impl PairFee {
    pub fn get_total_fee_percentage(&self) -> u64 {
        match self {
            PairFee::Percent04 => 40,  // 0.4%
            PairFee::Percent06 => 60,  // 0.6%
            PairFee::Percent10 => 100, // 1.0%
        }
    }
}
