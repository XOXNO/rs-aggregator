// ═══════════════════════════════════════════════════════════════════════════════
// Static Error Messages
// ═══════════════════════════════════════════════════════════════════════════════

pub const ERR_PREV_AMOUNT_NOT_AVAILABLE: &str = "PrevAmount not available";
pub const ERR_PREV_AMOUNT_TOKEN_MISMATCH: &str = "PrevAmount token mismatch";
pub const ERR_ZERO_INPUT_AMOUNT: &str = "Zero input amount";
pub const ERR_FEE_EXCEEDS_100: &str = "Fee exceeds 100%";
pub const ERR_REFERRAL_FEE_EXCEEDS_50: &str =
    "Referral fee exceeds 50% (total fees would exceed 100%)";
pub const ERR_REFERRAL_NOT_FOUND: &str = "Referral not found";
pub const ERR_NOT_REFERRAL_OWNER: &str = "Not referral owner";
pub const ERR_PPM_EXCEEDS_100_PERCENT: &str = "PPM value exceeds 1,000,000 (100%)";

// ═══════════════════════════════════════════════════════════════════════════════
// Dynamic Error Prefixes (token info appended at runtime)
// ═══════════════════════════════════════════════════════════════════════════════

pub const ERR_ONLY_FUNGIBLE_PREFIX: &[u8] = b"Only fungible ESDT tokens are accepted, got ";
pub const ERR_TOKEN_NOT_FOUND_PREFIX: &[u8] = b"Token not found in vault: ";
pub const ERR_INSUFFICIENT_BALANCE_PREFIX: &[u8] = b"Insufficient vault balance for token ";
