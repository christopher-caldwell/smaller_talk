use std::{collections::HashSet, path::PathBuf};

use async_trait::async_trait;
use smaller_talk_application::{AttachmentStorage, ExecutionError};
use smaller_talk_domain::Attachment;
use uuid::Uuid;

pub struct LocalAttachments {
    root: PathBuf,
}
impl LocalAttachments {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl AttachmentStorage for LocalAttachments {
    async fn import(&self, paths: Vec<PathBuf>) -> Result<Vec<Attachment>, ExecutionError> {
        if paths.len() > 100 {
            return Err(ExecutionError::new("Attach at most 100 files to a message"));
        }
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| ExecutionError::with_source("Could not create attachment storage", e))?;
        let mut result = Vec::new();
        let mut copied = Vec::new();
        let imported = async {
            let mut image_bytes = 0;
            for source in paths {
                let metadata = tokio::fs::metadata(&source).await.map_err(|e| {
                    ExecutionError::with_source(format!("Could not read {}", source.display()), e)
                })?;
                if !metadata.is_file() {
                    return Err(ExecutionError::new(format!(
                        "Folders cannot be attached: {}",
                        source.display()
                    )));
                }
                let name = source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| {
                        ExecutionError::new("The attachment filename is not valid Unicode")
                    })?
                    .to_owned();
                let mime = match source
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
                    "pdf" => "application/pdf",
                    "txt" | "md" | "rs" | "js" | "ts" | "tsx" | "csv" | "log" => "text/plain",
                    "json" => "application/json",
                    _ => "application/octet-stream",
                };
                let image = mime.starts_with("image/");
                let limit = if image {
                    10 * 1024 * 1024
                } else {
                    50 * 1024 * 1024
                };
                if metadata.len() > limit {
                    return Err(ExecutionError::new(format!(
                        "{name} exceeds the {} MiB file limit",
                        limit / 1024 / 1024
                    )));
                }
                if image {
                    image_bytes += metadata.len();
                }
                if image_bytes > 80 * 1024 * 1024 {
                    return Err(ExecutionError::new(
                        "Images in one message must total at most 80 MiB",
                    ));
                }
                let id = Uuid::new_v4().to_string();
                let extension = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .filter(|s| s.len() <= 12 && s.chars().all(|c| c.is_ascii_alphanumeric()));
                let target = self
                    .root
                    .join(extension.map_or_else(|| id.clone(), |e| format!("{id}.{e}")));
                copied.push(target.clone());
                // Keep the copy bounded even if another process grows the source during import.
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut reader = tokio::fs::File::open(&source)
                    .await
                    .map_err(|e| ExecutionError::with_source(format!("Could not open {name}"), e))?
                    .take(limit + 1);
                let mut writer = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not save attachment", e))?;
                let size = tokio::io::copy(&mut reader, &mut writer)
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not copy attachment", e))?;
                writer
                    .flush()
                    .await
                    .map_err(|e| ExecutionError::with_source("Could not flush attachment", e))?;
                if size > limit {
                    return Err(ExecutionError::new(format!(
                        "{name} grew beyond the file limit while being attached"
                    )));
                }
                let preview_path = if image {
                    let original = target.clone();
                    let preview = self.root.join(format!("{id}.thumb.png"));
                    copied.push(preview.clone());
                    let thumbnail = preview.clone();
                    tokio::task::spawn_blocking(move || -> Result<(), ExecutionError> {
                        let mut reader = image::ImageReader::open(original)
                            .map_err(|e| {
                                ExecutionError::with_source("Could not open the image", e)
                            })?
                            .with_guessed_format()
                            .map_err(|e| {
                                ExecutionError::with_source(
                                    "Could not identify the image format",
                                    e,
                                )
                            })?;
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(64 * 1024 * 1024);
                        limits.max_image_width = Some(16384);
                        limits.max_image_height = Some(16384);
                        reader.limits(limits);
                        let image = reader.decode().map_err(|e| {
                            ExecutionError::with_source(
                                "The image is invalid or too large to preview safely",
                                e,
                            )
                        })?;
                        image
                            .thumbnail(160, 120)
                            .save_with_format(thumbnail, image::ImageFormat::Png)
                            .map_err(|e| {
                                ExecutionError::with_source("Could not save the image thumbnail", e)
                            })?;
                        Ok(())
                    })
                    .await
                    .map_err(|e| ExecutionError::with_source("The thumbnail task stopped", e))??;
                    Some(preview)
                } else {
                    None
                };
                result.push(Attachment {
                    id,
                    name,
                    path: target,
                    mime_type: mime.into(),
                    byte_len: size,
                    preview_path,
                });
            }
            Ok(())
        }
        .await;
        if let Err(error) = imported {
            for path in copied {
                let _ = tokio::fs::remove_file(path).await;
            }
            return Err(error);
        }
        Ok(result)
    }

    async fn collect_unreferenced(&self, referenced: Vec<PathBuf>) -> Result<(), ExecutionError> {
        let keep: HashSet<_> = referenced.into_iter().collect();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(ExecutionError::with_source(
                    "Could not inspect attachment storage",
                    e,
                ));
            }
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ExecutionError::with_source("Could not inspect attachment", e))?
        {
            let path = entry.path();
            // Only remove UUID-named files owned by this store, never arbitrary neighboring files.
            let owned = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| Uuid::parse_str(s.split('.').next().unwrap_or("")).is_ok());
            if owned
                && !keep.contains(&path)
                && entry
                    .file_type()
                    .await
                    .map(|t| t.is_file())
                    .unwrap_or(false)
            {
                tokio::fs::remove_file(path).await.map_err(|e| {
                    ExecutionError::with_source("Could not remove unused attachment", e)
                })?;
            }
        }
        Ok(())
    }
}
