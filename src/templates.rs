use std::path::PathBuf;

use minijinja::{Environment, path_loader};

pub fn create_environment(schemas_dir: PathBuf) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_loader(path_loader("templates/"));
    env.add_function("get_nav_schemas", move || -> Vec<minijinja::Value> {
        let schemas = crate::schema::list_schemas(&schemas_dir).unwrap_or_default();
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
    env
}
