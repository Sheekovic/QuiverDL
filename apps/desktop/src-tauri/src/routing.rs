use std::path::{Component, Path, PathBuf};

#[tauri::command]
pub(crate) async fn resolve_smart_destination(
    default_path: String,
    category_folder: String,
    filename: String,
) -> Result<String, String> {
    let base = validate_absolute_folder(&default_path)?;
    let category = validate_relative_category(&category_folder)?;
    let filename = super::sanitize_filename(Some(filename.trim()));

    tokio::fs::create_dir_all(&base)
        .await
        .map_err(|error| format!("Could not create the default download folder: {error}"))?;
    let canonical_base = tokio::fs::canonicalize(&base)
        .await
        .map_err(|error| format!("Could not resolve the default download folder: {error}"))?;
    let category_path = canonical_base.join(category);
    tokio::fs::create_dir_all(&category_path)
        .await
        .map_err(|error| format!("Could not create the category folder: {error}"))?;
    let canonical_category = tokio::fs::canonicalize(&category_path)
        .await
        .map_err(|error| format!("Could not resolve the category folder: {error}"))?;
    if !canonical_category.starts_with(&canonical_base) {
        return Err("The category folder escapes the default download folder".into());
    }
    let destination = canonical_category.join(filename);
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(|error| format!("Could not inspect the smart destination: {error}"))?
    {
        return Err("The smart destination already exists; choose a different name".into());
    }
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) async fn resolve_category_directory(
    default_path: String,
    category_folder: String,
) -> Result<String, String> {
    let base = validate_absolute_folder(&default_path)?;
    let category = validate_relative_category(&category_folder)?;
    tokio::fs::create_dir_all(&base)
        .await
        .map_err(|error| format!("Could not create the default download folder: {error}"))?;
    let canonical_base = tokio::fs::canonicalize(&base)
        .await
        .map_err(|error| format!("Could not resolve the default download folder: {error}"))?;
    let category_path = canonical_base.join(category);
    tokio::fs::create_dir_all(&category_path)
        .await
        .map_err(|error| format!("Could not create the category folder: {error}"))?;
    let canonical_category = tokio::fs::canonicalize(&category_path)
        .await
        .map_err(|error| format!("Could not resolve the category folder: {error}"))?;
    if !canonical_category.starts_with(&canonical_base) {
        return Err("The category folder escapes the default download folder".into());
    }
    Ok(canonical_category.to_string_lossy().into_owned())
}

fn validate_absolute_folder(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.chars().count() > 4_096 || value.chars().any(char::is_control) {
        return Err("Choose a valid default download folder in Settings".into());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("The default download folder must be absolute".into());
    }
    Ok(path)
}

fn validate_relative_category(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty()
        || value.chars().count() > 240
        || value.chars().any(char::is_control)
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("The category folder must stay inside the default download folder".into());
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::validate_relative_category;

    #[test]
    fn category_paths_cannot_escape_the_default_folder() {
        assert!(validate_relative_category("Video/Clips").is_ok());
        assert!(validate_relative_category("   ").is_err());
        assert!(validate_relative_category("../Private").is_err());
        assert!(validate_relative_category("/absolute").is_err());
    }
}
