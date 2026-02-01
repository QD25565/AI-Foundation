#!/bin/bash
# Add dependencies for singleton pool optimization
cd "$(dirname "$0")"

# Add once_cell and parking_lot if not present
if ! grep -q "once_cell" Cargo.toml; then
    sed -i '/clap = { version = "4.4", features = \["derive"\] }/a once_cell = "1.19"\nparking_lot = "0.12"' Cargo.toml
    echo "Added once_cell and parking_lot dependencies"
else
    echo "Dependencies already present"
fi
