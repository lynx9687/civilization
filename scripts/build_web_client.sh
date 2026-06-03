cargo build --bin client --target wasm32-unknown-unknown
wasm-bindgen --no-typescript --target web \
    --out-dir ./web-client/ \
    --out-name "client" \
    ./target/wasm32-unknown-unknown/debug/client.wasm