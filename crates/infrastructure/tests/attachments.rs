use smaller_talk_application::AttachmentStorage;
use smaller_talk_infrastructure::LocalAttachments;

#[tokio::test]
async fn imported_images_have_bounded_thumbnails_and_survive_source_removal() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.png");
    image::RgbImage::from_pixel(640, 480, image::Rgb([30, 80, 220]))
        .save(&source)
        .unwrap();
    let storage = LocalAttachments::new(temp.path().join("attachments"));
    let files = storage.import(vec![source.clone()]).await.unwrap();
    std::fs::remove_file(source).unwrap();
    assert!(files[0].path.is_file());
    let preview = files[0].preview_path.as_ref().unwrap();
    let dimensions = image::image_dimensions(preview).unwrap();
    assert!(dimensions.0 <= 160 && dimensions.1 <= 120);
    storage
        .collect_unreferenced(vec![files[0].path.clone(), preview.clone()])
        .await
        .unwrap();
    assert!(preview.is_file());
    storage.collect_unreferenced(vec![]).await.unwrap();
    assert!(!preview.exists());
    assert!(!files[0].path.exists());
}

#[tokio::test]
async fn invalid_batch_rolls_back_copies_and_never_removes_source_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.txt");
    std::fs::write(&source, "context").unwrap();
    let root = temp.path().join("attachments");
    let storage = LocalAttachments::new(root.clone());
    let result = storage
        .import(vec![source.clone(), temp.path().to_owned()])
        .await;
    assert!(result.unwrap_err().to_string().contains("Folders"));
    assert!(source.is_file());
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}
