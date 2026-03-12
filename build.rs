fn main() {
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if profile == "release" {
        minijinja_embed::embed_templates!("templates");
    }
}
