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
    JexStableRemoveLiquidity(u32), // Count of output tokens

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

    /// Returns owner_fee + real_yield_fee (the portion that leaves the pool)
    pub fn get_special_fee_percentage(&self) -> u64 {
        match self {
            PairFee::Percent04 => 20, // 0.1% owner + 0.1% real_yield = 0.2%
            PairFee::Percent06 => 30, // 0.15% + 0.15% = 0.3%
            PairFee::Percent10 => 50, // 0.25% + 0.25% = 0.5%
        }
    }
}

/// Referral configuration stored per referral ID
#[type_abi]
#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone)]
pub struct ReferralConfig<M: ManagedTypeApi> {
    pub owner: ManagedAddress<M>,
    pub fee: u32, // basis points (10,000 = 100%)
    pub active: bool,
}

// =============================================================================
// Compact Encoding Types (for efficient transaction payloads)
// =============================================================================

/// Compact action type as u8 for minimal encoding
/// Maps to ActionType variants but without embedded data
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CompactAction {
    // xExchange (0-2)
    XExchangeSwap = 0,
    XExchangeAddLiquidity = 1,
    XExchangeRemoveLiquidity = 2,
    // AshSwap V1 (3-5)
    AshSwapPoolSwap = 3,
    AshSwapPoolAddLiquidity = 4,
    AshSwapPoolRemoveLiquidity = 5,
    // AshSwap V2 (6-8)
    AshSwapV2Swap = 6,
    AshSwapV2AddLiquidity = 7,
    AshSwapV2RemoveLiquidity = 8,
    // OneDex (9-11)
    OneDexSwap = 9,
    OneDexAddLiquidity = 10,
    OneDexRemoveLiquidity = 11,
    // Jex CPMM (12-14)
    JexSwap = 12,
    JexAddLiquidity = 13,
    JexRemoveLiquidity = 14,
    // Jex Stable (15-17)
    JexStableSwap = 15,
    JexStableAddLiquidity = 16,
    JexStableRemoveLiquidity = 17,
    // EGLD wrapping (18-19)
    Wrapping = 18,
    UnWrapping = 19,
    // Liquid staking (20-22)
    XoxnoLiquidStaking = 20,
    LXoxnoLiquidStaking = 21,
    HatomLiquidStaking = 22,
    // Hatom (23-24)
    HatomRedeem = 23,
    HatomSupply = 24,
}

impl CompactAction {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::XExchangeSwap),
            1 => Some(Self::XExchangeAddLiquidity),
            2 => Some(Self::XExchangeRemoveLiquidity),
            3 => Some(Self::AshSwapPoolSwap),
            4 => Some(Self::AshSwapPoolAddLiquidity),
            5 => Some(Self::AshSwapPoolRemoveLiquidity),
            6 => Some(Self::AshSwapV2Swap),
            7 => Some(Self::AshSwapV2AddLiquidity),
            8 => Some(Self::AshSwapV2RemoveLiquidity),
            9 => Some(Self::OneDexSwap),
            10 => Some(Self::OneDexAddLiquidity),
            11 => Some(Self::OneDexRemoveLiquidity),
            12 => Some(Self::JexSwap),
            13 => Some(Self::JexAddLiquidity),
            14 => Some(Self::JexRemoveLiquidity),
            15 => Some(Self::JexStableSwap),
            16 => Some(Self::JexStableAddLiquidity),
            17 => Some(Self::JexStableRemoveLiquidity),
            18 => Some(Self::Wrapping),
            19 => Some(Self::UnWrapping),
            20 => Some(Self::XoxnoLiquidStaking),
            21 => Some(Self::LXoxnoLiquidStaking),
            22 => Some(Self::HatomLiquidStaking),
            23 => Some(Self::HatomRedeem),
            24 => Some(Self::HatomSupply),
            _ => None,
        }
    }

    /// Check if this action needs an output token parameter
    pub fn needs_output_token(&self) -> bool {
        matches!(
            self,
            Self::XExchangeSwap
                | Self::AshSwapPoolSwap
                | Self::OneDexSwap
                | Self::JexStableSwap
                | Self::HatomSupply
        )
    }

    /// Check if this is an add_liquidity action that can be ZAPped
    pub fn is_zappable(&self) -> bool {
        matches!(
            self,
            Self::XExchangeAddLiquidity | Self::OneDexAddLiquidity | Self::JexAddLiquidity
        )
    }

    /// Check if this is a stable/multi-asset add_liquidity (supports 3+ inputs)
    /// Format: tok1, tok2, tok3, shared_mode, address
    pub fn is_multi_input_add_liquidity(&self) -> bool {
        matches!(
            self,
            Self::AshSwapPoolAddLiquidity
                | Self::AshSwapV2AddLiquidity
                | Self::JexStableAddLiquidity
        )
    }

    /// Check if this action needs output count (for remove liquidity)
    /// Format: [action, count, in_tok, in_mode, 0, addr]
    pub fn needs_output_count(&self) -> bool {
        matches!(
            self,
            Self::AshSwapPoolRemoveLiquidity
                | Self::AshSwapV2RemoveLiquidity
                | Self::JexStableRemoveLiquidity
        )
    }

    /// Check if this action needs pair_id (OneDex add liquidity)
    /// Format: [action, pair_id, tok1, mode1, tok2, mode2]
    pub fn needs_pair_id(&self) -> bool {
        matches!(self, Self::OneDexAddLiquidity)
    }
}

/// Compact amount mode as u8
/// 0 = All, 1 = Prev, 2-127 = Fixed amount index, 128-255 = PPM index
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CompactMode {
    All,
    Prev,
    Fixed(u8), // Index into amounts registry (amounts[idx] is the exact amount)
    Ppm(u8),   // Index into amounts registry (amounts[idx] is the PPM value)
}

/// Threshold for PPM mode (values >= this are PPM indices)
pub const MODE_PPM_THRESHOLD: u8 = 128;

impl CompactMode {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::All,
            1 => Self::Prev,
            v if v >= MODE_PPM_THRESHOLD => Self::Ppm(v - MODE_PPM_THRESHOLD),
            v => Self::Fixed(v - 2), // 2-127 → amounts[0-125]
        }
    }
}

/// Special index values for compact encoding
pub const IDX_NONE: u8 = 255;
pub const IDX_EGLD: u8 = 254;
pub const IDX_AUTO: u8 = 255;
