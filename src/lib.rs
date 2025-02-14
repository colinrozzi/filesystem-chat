// New version
impl State {
    fn resolve_path(&self, relative_path: &str) -> String {
        if relative_path.starts_with("/") {
            // Don't modify absolute paths
            relative_path.to_string()
        } else {
            // Join the base path with the relative path
            let base_path = &self.fs_path;
            if relative_path == "." {
                base_path.to_string()
            } else {
                format!("{}/{}", base_path, relative_path)
            }
        }
    }
}