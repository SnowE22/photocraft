//! Ask Google about Nano Banana 2.1 with the saved API key (free: nothing is generated), to check
//! a setup. The key itself is never printed.
//!
//! ```sh
//! cargo run -p photocraft-ml --example nano_banana_check
//! ```

fn main() {
    println!("key: {:?}", photocraft_ml::gemini::key_source());
    match photocraft_ml::gemini::check() {
        Ok(answer) => println!("{answer}"),
        Err(e) => println!("error: {e}"),
    }
}
