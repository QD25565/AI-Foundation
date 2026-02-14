// Build script for UniFFI scaffolding generation
fn main() {
    uniffi::generate_scaffolding("src/deepnet.udl").unwrap();
}
