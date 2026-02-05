ADDRESS=erd1qqqqqqqqqqqqqpgq5rf2sppxk2xu4m0pkmugw2es4gak3rgjah0sxvajva
PROXY=https://gateway.xoxno.com
PROJECT="./output/aggregator.wasm"

deploy() {
    mxpy contract deploy --bytecode=${PROJECT} --metadata-payable-by-sc \
    --ledger \
    --gas-limit=150000000 --send --proxy=${PROXY} --chain=1 || return

    echo "New smart contract address: ${ADDRESS}"
}

upgrade() {
    echo "Upgrade smart contract address: ${ADDRESS}"
    mxpy  contract upgrade ${ADDRESS} --bytecode=${PROJECT} \
    --ledger \
    --gas-limit=150000000 --send --proxy=${PROXY} --chain=1 || return
}

# --- Config Endpoints ---

# Add a new referral with the given owner and fee (only owner)
# Usage: addReferral <owner_address> <fee>
# fee is in basis points (e.g., 100 = 1%, 500 = 5%, 10000 = 100%)
addReferral() {
    owner=$1
    fee=$2
    mxpy contract call ${ADDRESS} --function=addReferral \
    --arguments ${owner} ${fee} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Update the fee for an existing referral (only owner)
# Usage: setReferralFee <referral_id> <fee>
setReferralFee() {
    referral_id=$1
    fee=$2
    mxpy contract call ${ADDRESS} --function=setReferralFee \
    --arguments ${referral_id} ${fee} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Enable or disable a referral (only owner)
# Usage: setReferralActive <referral_id> <active>
# active: true or false
setReferralActive() {
    referral_id=$1
    active=$2
    mxpy contract call ${ADDRESS} --function=setReferralActive \
    --arguments ${referral_id} ${active} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Change the owner of an existing referral (only owner)
# Usage: setReferralOwner <referral_id> <new_owner_address>
setReferralOwner() {
    referral_id=$1
    new_owner=$2
    mxpy contract call ${ADDRESS} --function=setReferralOwner \
    --arguments ${referral_id} ${new_owner} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Set the static fee for trades without a referral (only owner)
# Usage: setStaticFee <fee>
# fee is in basis points (e.g., 100 = 1%, 500 = 5%, 10000 = 100%)
setStaticFee() {
    fee=$1
    mxpy contract call ${ADDRESS} --function=setStaticFee \
    --arguments ${fee} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Claim accumulated admin fees (only owner)
# Usage: claimAdminFees <recipient_address>
claimAdminFees() {
    recipient=$1
    mxpy contract call ${ADDRESS} --function=claimAdminFees \
    --arguments addr:${recipient} \
    --ledger --send --proxy=${PROXY} --chain=1
}

# Claim accumulated referral fees (can be called by referral owner)
# Usage: claimReferralFees <referral_id>
claimReferralFees() {
    referral_id=$1
    mxpy contract call ${ADDRESS} --function=claimReferralFees \
    --arguments ${referral_id} \
    --ledger --send --proxy=${PROXY} --chain=1
}

# --- Fee Whitelist Endpoints ---

# Add a token to the fee whitelist (only owner)
# Whitelisted tokens trigger fee-on-output when used as output token
# Usage: addWhitelistedToken <token_identifier>
# Example: addWhitelistedToken WEGLD-bd4d79
addWhitelistedToken() {
    token=$1
    mxpy contract call ${ADDRESS} --function=addWhitelistedToken \
    --arguments str:${token} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Remove a token from the fee whitelist (only owner)
# Usage: removeWhitelistedToken <token_identifier>
removeWhitelistedToken() {
    token=$1
    mxpy contract call ${ADDRESS} --function=removeWhitelistedToken \
    --arguments str:${token} \
    --ledger \
    --gas-limit=10000000 --send --proxy=${PROXY} --chain=1
}

# Batch add multiple tokens to the fee whitelist (only owner)
# Usage: addWhitelistedTokensBatch <token1> <token2> ...
# Example: addWhitelistedTokensBatch WEGLD-bd4d79 USDC-c76f1f
addWhitelistedTokensBatch() {
    local args=()
    for token in "$@"; do
        args+=("str:${token}")
    done
    mxpy contract call ${ADDRESS} --function=addWhitelistedTokensBatch \
    --arguments "${args[@]}" \
    --ledger \
    --gas-limit=20000000 --send --proxy=${PROXY} --chain=1
}

# Clear all whitelisted tokens (only owner)
# Usage: clearWhitelistedTokens
clearWhitelistedTokens() {
    mxpy contract call ${ADDRESS} --function=clearWhitelistedTokens \
    --ledger \
    --gas-limit=50000000 --send --proxy=${PROXY} --chain=1
}

# Query whitelisted tokens (view function)
# Usage: getWhitelistedTokens
getWhitelistedTokens() {
    mxpy contract query ${ADDRESS} --function=getWhitelistedTokens --proxy=${PROXY}
}
