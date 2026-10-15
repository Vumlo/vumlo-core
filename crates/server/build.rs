// `sqlx::migrate!` embeds the migration files at compile time. Without this, adding a new
// file to `migrations/` would not trigger a rebuild, and the old set would stay embedded.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
