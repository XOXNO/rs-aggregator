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
    mxpy  contract upgrade ${ADDRESS} --metadata-payable-by-sc --bytecode=${PROJECT} \
    --ledger \
    --gas-limit=150000000 --send --proxy=${PROXY} --chain=1 || return
}