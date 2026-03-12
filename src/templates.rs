use std::path::PathBuf;

use minijinja::{Environment, path_loader};
use minijinja_autoreload::AutoReloader;

pub fn create_reloader(schemas_dir: PathBuf) -> AutoReloader {
    AutoReloader::new(move |notifier| {
        let mut env = Environment::new();
        env.set_loader(path_loader("templates/"));
        // Default base_template — overridden to "_partial.html" for htmx requests
        env.add_global("base_template", minijinja::Value::from("base.html"));
        let sd = schemas_dir.clone();
        env.add_function("get_nav_schemas", move || -> Vec<minijinja::Value> {
            let schemas = crate::schema::list_schemas(&sd).unwrap_or_default();
            schemas
                .iter()
                .map(|s| {
                    minijinja::context! {
                        title => s.meta.title,
                        slug => s.meta.slug,
                    }
                })
                .collect()
        });
        notifier.set_fast_reload(true);
        Ok(env)
    })

}

/// Returns the base template name based on whether this is an htmx request.
pub fn base_for_htmx(is_htmx: bool) -> &'static str {
    if is_htmx {
        "_partial.html"
    } else {
        "base.html"
    }
}
