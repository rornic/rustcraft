pub mod primitives;

use std::fs::read_to_string;
use std::fs::File;

/// Opens a resource file in the `resources` folder.
pub fn get_resource_file(name: &str) -> Result<File, std::io::Error> {
    File::open(format!("resources/{}", name))
}

/// Reads the contents of a resource file as a `String`.
pub fn get_resource_file_as_string(name: &str) -> Result<String, std::io::Error> {
    read_to_string(format!("resources/{}", name))
}
