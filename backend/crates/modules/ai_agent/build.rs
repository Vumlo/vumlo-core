// `sqlx::migrate!` embeds the migration files at compile time; see crates/db/build.rs.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
