use std::fs;
use std::path::Path;

pub fn create_dir_unless_exists(directory: &Path) {
    match fs::create_dir_all(directory) {
        Ok(()) => {},
        Err(e) => {
            panic!("Unexpected I/O error occurred: {:?}", e);
        }
    }
}