use smaller_talk_application::{ExecutionError, SteerInput};

pub async fn validate(input: &SteerInput) -> Result<(), ExecutionError> {
    if input.attachments.len() > 100 {
        return Err(ExecutionError::new("Attach at most 100 files."));
    }
    let mut image_bytes = 0;
    for file in &input.attachments {
        let metadata = tokio::fs::metadata(&file.path).await.map_err(|e| {
            ExecutionError::with_source(format!("Attachment {} is unavailable", file.name), e)
        })?;
        if !metadata.is_file() {
            return Err(ExecutionError::new(format!(
                "Attachment {} is not a file",
                file.name
            )));
        }
        let _ = tokio::fs::File::open(&file.path).await.map_err(|e| {
            ExecutionError::with_source(format!("Attachment {} is unreadable", file.name), e)
        })?;
        if file.is_image() {
            image_bytes += metadata.len();
            if metadata.len() > 10 * 1024 * 1024 {
                return Err(ExecutionError::new(format!(
                    "Image {} exceeds 10 MiB",
                    file.name
                )));
            }
        }
    }
    if image_bytes > 80 * 1024 * 1024 {
        return Err(ExecutionError::new("Images exceed 80 MiB."));
    }
    Ok(())
}
