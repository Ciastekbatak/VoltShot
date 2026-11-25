pub mod color;
pub mod combiner;
pub mod hash;
pub mod indexer;
pub mod ocr;
pub mod ocr_winrt;
pub mod qr;
pub mod ruler;

pub use color::{ColorSample, sample_image};
pub use combiner::{
    CombineAlignment, CombineOptions, CombineOrientation, ContactSheetOptions, combine_images,
    generate_contact_sheet,
};
pub use hash::{HashAlgorithm, compute_bytes_hash, compute_file_hash, verify_hash};
pub use indexer::{DirectoryNode, IndexFormat, IndexOptions, render_index, scan_directory};
pub use ocr::{
    FallbackOcrChain, OcrEngine, OcrOptions, OcrResult, TesseractCliOcr, build_tesseract_args,
    find_tesseract,
};
pub use ocr_winrt::{WinRtOcr, normalize_bcp47_language_tag};
pub use qr::{decode_qr_image, generate_qr_image};
pub use ruler::RulerMeasurement;
