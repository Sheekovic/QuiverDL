use std::path::{Component, Path, PathBuf};

#[tauri::command]
pub(crate) fn default_download_directory() -> Result<String, String> {
    dirs::download_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or("Could not find Downloads".into())
}

#[tauri::command]
pub(crate) async fn resolve_browser_destination(
    default_path: String,
    category_folder: String,
    filename: String,
) -> Result<String, String> {
    let base = if default_path.is_empty() {
        dirs::download_dir().ok_or("Could not locate the Downloads folder")?
    } else {
        validate_absolute_folder(&default_path)?
    };
    let directory = resolve_category_directory(
        base.to_string_lossy().into_owned(),
        if category_folder.is_empty() {
            ".".into()
        } else {
            category_folder
        },
    )
    .await?;
    let filename = super::sanitize_filename(Some(&filename));
    let original = Path::new(&filename);
    let stem = original.file_stem().unwrap_or_default().to_string_lossy();
    let extension = original
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            filename.clone()
        } else {
            format!("{stem} ({suffix}){extension}")
        };
        let destination = Path::new(&directory).join(name);
        let busy = [
            destination.clone(),
            PathBuf::from(format!("{}.quiver-part", destination.display())),
            PathBuf::from(format!("{}.quiver.json", destination.display())),
        ];
        if busy.iter().all(|path| !path.exists()) {
            return Ok(destination.to_string_lossy().into_owned());
        }
    }
    Err("Could not choose an unused download filename".into())
}

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
