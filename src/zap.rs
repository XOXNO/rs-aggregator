multiversx_sc::imports!();

// ZAP mathematics for optimal liquidity provision
//
// Pre-balances two token amounts before add_liquidity to minimize dust.
// Uses binary search to find the optimal swap amount that results in
// perfectly balanced tokens for the pool's current reserves.

// Maximum iterations for binary search (128 ensures convergence for high-precision tokens)
pub const MAX_BINARY_SEARCH_ITERATIONS: u32 = 128;

/// Fee application mode for different DEXes
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeeMode {
    /// Fee applied to input amount (xExchange, OneDex)
    /// output = (input * fee_factor * reserve_out) / (reserve_in * fee_denom + input * fee_factor)
    OnInput,
    /// Fee applied to output amount (JEX)
    /// raw_output = (input * reserve_out) / (reserve_in + input)
    /// output = raw_output * fee_factor / fee_denom
    OnOutput,
}

/// Simulate swap output for constant product AMM (no actual execution)
///
/// # Arguments
/// * `amount_in` - Amount of input token to swap
/// * `reserve_in` - Reserve of input token in the pool
/// * `reserve_out` - Reserve of output token in the pool
/// * `fee_num` - Fee numerator (e.g., 300 for 0.3% on xExchange)
/// * `fee_denom` - Fee denominator (e.g., 100_000 for xExchange)
/// * `fee_mode` - Whether fee is applied on input or output
///
/// # Returns
/// (output_amount, raw_output_before_fee) - raw_output is needed for reserve calculation
pub fn simulate_swap_output<M: ManagedTypeApi>(
    amount_in: &BigUint<M>,
    reserve_in: &BigUint<M>,
    reserve_out: &BigUint<M>,
    fee_num: u64,
    fee_denom: u64,
    fee_mode: FeeMode,
) -> (BigUint<M>, BigUint<M>) {
    if amount_in == &BigUint::zero()
        || reserve_in == &BigUint::zero()
        || reserve_out == &BigUint::zero()
    {
        return (BigUint::zero(), BigUint::zero());
    }

    let fee_factor = fee_denom - fee_num;

    match fee_mode {
        FeeMode::OnInput => {
            // xExchange/OneDex: fee applied to input
            // output = (input * fee_factor * reserve_out) / (reserve_in * fee_denom + input * fee_factor)
            let numerator = amount_in * fee_factor * reserve_out;
            let denominator = reserve_in * fee_denom + amount_in * fee_factor;
            let output = &numerator / &denominator;
            (output.clone(), output)
        }
        FeeMode::OnOutput => {
            // JEX: fee applied to output
            // raw_output = (input * reserve_out) / (reserve_in + input)
            // output = raw_output * fee_factor / fee_denom
            let numerator = amount_in * reserve_out;
            let denominator = reserve_in + amount_in;
            let raw_output = &numerator / &denominator;
            let output = &raw_output * fee_factor / fee_denom;
            (output, raw_output)
        }
    }
}

/// Given two token balances and pool state, compute optimal swap to balance them
/// for add_liquidity. This is called BEFORE add_liquidity to pre-balance tokens.
///
/// # Arguments
/// * `balance_first` - Our balance of the pool's first token
/// * `balance_second` - Our balance of the pool's second token
/// * `reserve_first` - Pool's reserve of first token
/// * `reserve_second` - Pool's reserve of second token
/// * `fee_num` - Fee numerator
/// * `fee_denom` - Fee denominator
/// * `fee_mode` - Whether fee is applied on input or output
///
/// # Returns
/// (swap_from_first, swap_amount):
/// - If swap_from_first is true: swap `swap_amount` of first token for second
/// - If swap_from_first is false: swap `swap_amount` of second token for first
/// - If swap_amount is 0: tokens are already balanced
pub fn compute_optimal_pre_swap<M: ManagedTypeApi>(
    balance_first: &BigUint<M>,
    balance_second: &BigUint<M>,
    reserve_first: &BigUint<M>,
    reserve_second: &BigUint<M>,
    fee_num: u64,
    fee_denom: u64,
    fee_mode: FeeMode,
) -> (bool, BigUint<M>) {
    // Edge cases
    if balance_first == &BigUint::zero()
        || balance_second == &BigUint::zero()
        || reserve_first == &BigUint::zero()
        || reserve_second == &BigUint::zero()
    {
        return (true, BigUint::zero());
    }

    // Check which token is in excess by comparing ratios:
    // balance_first / reserve_first vs balance_second / reserve_second
    // Cross multiply: balance_first * reserve_second vs balance_second * reserve_first
    let product_first = balance_first * reserve_second;
    let product_second = balance_second * reserve_first;

    if product_first > product_second {
        // First token is in excess, need to swap some first → second
        let swap_amount = binary_search_pre_swap(
            balance_first,
            balance_second,
            reserve_first,
            reserve_second,
            fee_num,
            fee_denom,
            fee_mode,
            true, // swapping from first
        );
        (true, swap_amount)
    } else if product_second > product_first {
        // Second token is in excess, need to swap some second → first
        let swap_amount = binary_search_pre_swap(
            balance_first,
            balance_second,
            reserve_first,
            reserve_second,
            fee_num,
            fee_denom,
            fee_mode,
            false, // swapping from second
        );
        (false, swap_amount)
    } else {
        // Already balanced
        (true, BigUint::zero())
    }
}

/// Binary search to find optimal swap amount for pre-balancing two token balances
#[allow(clippy::too_many_arguments)]
fn binary_search_pre_swap<M: ManagedTypeApi>(
    balance_first: &BigUint<M>,
    balance_second: &BigUint<M>,
    reserve_first: &BigUint<M>,
    reserve_second: &BigUint<M>,
    fee_num: u64,
    fee_denom: u64,
    fee_mode: FeeMode,
    swap_from_first: bool,
) -> BigUint<M> {
    // Determine which balance we're swapping from
    let (swap_balance, other_balance, reserve_in, reserve_out) = if swap_from_first {
        (balance_first, balance_second, reserve_first, reserve_second)
    } else {
        (balance_second, balance_first, reserve_second, reserve_first)
    };

    let mut low = BigUint::zero();
    let mut high = swap_balance.clone();

    for _ in 0..MAX_BINARY_SEARCH_ITERATIONS {
        // Check convergence
        if high <= &low + 1u64 {
            break;
        }

        // Safe midpoint
        let mid = &low + &((&high - &low) / 2u64);

        // Simulate swap at midpoint
        let (received, raw_output) =
            simulate_swap_output(&mid, reserve_in, reserve_out, fee_num, fee_denom, fee_mode);

        if received == BigUint::zero() {
            low = mid;
            continue;
        }

        // Calculate final balances after swap
        let final_swap_balance = swap_balance - &mid;
        let final_other_balance = other_balance + &received;

        // Calculate new reserves after swap
        let new_reserve_in = reserve_in + &mid;
        let new_reserve_out = reserve_out - &raw_output;

        // Check if final balances are in ratio with new reserves
        // final_swap_balance / new_reserve_in vs final_other_balance / new_reserve_out
        // Cross multiply: final_swap_balance * new_reserve_out vs final_other_balance * new_reserve_in
        let product_swap = &final_swap_balance * &new_reserve_out;
        let product_other = &final_other_balance * &new_reserve_in;

        if product_swap > product_other {
            // Still have excess of swap token, need to swap more
            low = mid;
        } else if product_swap < product_other {
            // Swapped too much, have excess of other token now
            high = mid;
        } else {
            // Perfect balance
            return mid;
        }
    }

    // Return best candidate
    &low + &((&high - &low) / 2u64)
}
