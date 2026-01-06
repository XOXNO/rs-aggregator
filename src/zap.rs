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
    ///
    /// For xExchange: special_fee leaves pool (burned/sent to fees collector)
    /// reserve_in += (input - special_fee), NOT full input
    OnInput { special_fee_num: u64 },
    /// Fee applied to output amount with split fees (JEX)
    ///
    /// - lp_fee stays in pool (affects reserves)
    /// - protocol_fee leaves pool (doesn't affect reserves)
    ///
    /// Formulas:
    /// - raw_output = (input * reserve_out) / (reserve_in + input)
    /// - output = raw_output * (fee_denom - total_fee) / fee_denom
    /// - amount_leaving_pool = raw_output * (fee_denom - lp_fee) / fee_denom
    ///
    /// lp_fee_num: The LP portion of the fee that stays in the pool
    OnOutput { lp_fee_num: u64 },
}

/// Simulate swap output for constant product AMM (no actual execution)
///
/// # Arguments
/// * `amount_in` - Amount of input token to swap
/// * `reserve_in` - Reserve of input token in the pool
/// * `reserve_out` - Reserve of output token in the pool
/// * `fee_num` - Total fee numerator (e.g., 300 for 0.3% on xExchange)
/// * `fee_denom` - Fee denominator (e.g., 100_000 for xExchange)
/// * `fee_mode` - Whether fee is applied on input or output (with fee split info)
///
/// # Returns
/// (output_amount, amount_out_leaving, amount_in_to_reserves)
pub fn simulate_swap_output<M: ManagedTypeApi>(
    amount_in: &BigUint<M>,
    reserve_in: &BigUint<M>,
    reserve_out: &BigUint<M>,
    fee_num: u64,
    fee_denom: u64,
    fee_mode: FeeMode,
) -> (BigUint<M>, BigUint<M>, BigUint<M>) {
    if amount_in == &BigUint::zero()
        || reserve_in == &BigUint::zero()
        || reserve_out == &BigUint::zero()
    {
        return (BigUint::zero(), BigUint::zero(), BigUint::zero());
    }

    // Safety check: fee_num should not exceed fee_denom
    if fee_num > fee_denom {
        return (BigUint::zero(), BigUint::zero(), BigUint::zero());
    }

    let fee_factor = fee_denom - fee_num;

    match fee_mode {
        FeeMode::OnInput { special_fee_num } => {
            // xExchange/OneDex: fee applied to input
            // output = (input * fee_factor * reserve_out) / (reserve_in * fee_denom + input * fee_factor)
            let numerator = amount_in * fee_factor * reserve_out;
            let denominator = reserve_in * fee_denom + amount_in * fee_factor;
            let output = &numerator / &denominator;

            // For xExchange: special_fee leaves the pool (burned/sent to fees collector)
            // amount_in_to_reserves = amount_in - special_fee
            // For OneDex: special_fee_num = 0, so full amount goes to reserves
            let special_fee = amount_in * special_fee_num / fee_denom;
            let amount_in_to_reserves = amount_in - &special_fee;

            (output.clone(), output, amount_in_to_reserves)
        }
        FeeMode::OnOutput { lp_fee_num } => {
            // JEX: fee applied to output with split fees
            // raw_output = (input * reserve_out) / (reserve_in + input)
            // output = raw_output * (fee_denom - total_fee) / fee_denom  (what user gets)
            // amount_leaving = raw_output * (fee_denom - lp_fee) / fee_denom  (what leaves pool)
            //
            // LP fee stays in pool, protocol fee + user output leaves pool
            let numerator = amount_in * reserve_out;
            let denominator = reserve_in + amount_in;
            let raw_output = &numerator / &denominator;

            // What user receives (after all fees)
            let output = &raw_output * fee_factor / fee_denom;

            // What actually leaves the pool reserves (raw_output minus LP fee that stays)
            // amount_leaving = raw_output - (raw_output * lp_fee_num / fee_denom)
            //                = raw_output * (fee_denom - lp_fee_num) / fee_denom
            let lp_fee_factor = fee_denom - lp_fee_num;
            let amount_out_leaving = &raw_output * lp_fee_factor / fee_denom;

            // For OnOutput, all input goes to reserves
            (output, amount_out_leaving, amount_in.clone())
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
/// - If swap_amount is 0: tokens are already perfectly balanced

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

    // NOTE: We intentionally do NOT have a tolerance check here.
    // Even when tokens are "nearly balanced", the SC's quote() uses truncated
    // integer division which creates dust. We must always compute the optimal
    // swap amount to minimize dust, regardless of how close the ratios are.

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

/// Binary search to find optimal swap amount for pre-balancing two token balances.
///
/// The goal is to minimize dust returned by the SC's add_liquidity function.
/// The SC uses `quote()` which truncates: `optimal_b = a * reserve_b / reserve_a`
///
/// We search for the swap amount where the SC's quote calculation results in
/// minimal leftover (dust). The SC will use all of one token and return dust of the other.
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
    let mut best_swap = BigUint::zero();
    let mut best_dust = swap_balance.clone(); // Start with worst case

    for _ in 0..MAX_BINARY_SEARCH_ITERATIONS {
        // Check convergence
        if high <= &low + 1u64 {
            break;
        }

        // Safe midpoint
        let mid = &low + &((&high - &low) / 2u64);

        // Simulate swap at midpoint
        // Returns (user_output, amount_out_leaving, amount_in_to_reserves)
        let (received, amount_out_leaving, amount_in_to_reserves) =
            simulate_swap_output(&mid, reserve_in, reserve_out, fee_num, fee_denom, fee_mode);

        if received == BigUint::zero() {
            low = mid;
            continue;
        }

        // Calculate final balances after swap
        let final_swap_balance = swap_balance - &mid;
        let final_other_balance = other_balance + &received;

        // Calculate new reserves after swap
        let new_reserve_in = reserve_in + &amount_in_to_reserves;
        let new_reserve_out = reserve_out - &amount_out_leaving;

        // Simulate SC's set_optimal_amounts logic using quote()
        // quote(a, res_a, res_b) = a * res_b / res_a (truncated)
        // SC checks: if quote(swap_bal, new_res_in, new_res_out) <= other_bal
        //   then use (swap_bal, quote_result) -> dust = other_bal - quote_result
        //   else use (quote(other_bal, new_res_out, new_res_in), other_bal) -> dust = swap_bal - quote_result

        let quote_other_from_swap = &final_swap_balance * &new_reserve_out / &new_reserve_in;

        let dust = if &quote_other_from_swap <= &final_other_balance {
            // SC will use all of swap_balance, return excess other_balance
            &final_other_balance - &quote_other_from_swap
        } else {
            // SC will use all of other_balance, return excess swap_balance
            let quote_swap_from_other = &final_other_balance * &new_reserve_in / &new_reserve_out;
            &final_swap_balance - &quote_swap_from_other
        };

        // Track best result
        if dust < best_dust {
            best_dust = dust.clone();
            best_swap = mid.clone();
        }

        // Binary search direction based on ratio comparison
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

    // Return the swap amount that minimizes dust
    best_swap
}
