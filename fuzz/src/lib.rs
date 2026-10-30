/// Write the user-facing units, independently of the canonical numeric JSON.
pub fn app_toml(spec: &appspec::AppSpec) -> String {
    let mut value = toml::Value::try_from(spec).unwrap();
    let mut limits = toml::map::Map::new();
    if let Some(cpu) = spec.limits.cpu_millicores {
        limits.insert("cpu".into(), toml::Value::Float(f64::from(cpu) / 1000.0));
    }
    if let Some(bytes) = spec.limits.memory_bytes {
        limits.insert(
            "memory".into(),
            toml::Value::String(format!("{} MiB", bytes >> 20)),
        );
    }
    if let Some(bytes) = spec.limits.disk_bytes {
        limits.insert(
            "disk".into(),
            toml::Value::String(format!("{} MiB", bytes >> 20)),
        );
    }
    value
        .as_table_mut()
        .unwrap()
        .insert("limits".into(), toml::Value::Table(limits));
    toml::to_string(&value).unwrap()
}
