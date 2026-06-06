# Build client for web assemply. Only client package is built for this target
cargo build -p client --target wasm32-unknown-unknown
# Create javascripty bindings
wasm-bindgen --no-typescript --target web \
    --out-dir ./web-client/ \
    --out-name "client" \
    ./target/wasm32-unknown-unknown/debug/client.wasm
# Copy assets to web-client directory
cp -r assets/* ./web-client/assets/