#[cfg(debug_assertions)]
static MIRROR_LIST_FILE: &str = "./etc/flexo/mirrorlist";

#[cfg(not(debug_assertions))]
static MIRROR_LIST_FILE: &str = "/etc/flexo/mirrorlist";

// Fallback strategy in case the JSON endpoint cannot be reached: The selected mirrors are stored in a text file
// so that we can simply retrieve and reuse the previously selected mirrors from this file, instead of fetching
// the mirrors from the JSON endpoint.

pub fn store(mirrors: &Vec<String>) {
    let data = mirrors.join("\n");
    std::fs::write(MIRROR_LIST_FILE, data)
        .unwrap_or_else(|_| panic!("Unable to write file: {}", MIRROR_LIST_FILE));
}

pub fn fetch() -> Result<Vec<String>, std::io::Error> {
    let contents = std::fs::read_to_string(MIRROR_LIST_FILE)?;
    Ok(contents.split("\n").map(|s| s.to_owned()).collect())
}
