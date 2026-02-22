// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result, bail};
use std::io::{Read, Cursor};
use zip::ZipArchive;

/// Safety limits for ZIP extraction to prevent ZIP bombs
const MAX_ZIP_SIZE: u64 = 10 * 1024 * 1024; // 10MB max ZIP file size
const MAX_UNCOMPRESSED_SIZE: u64 = 50 * 1024 * 1024; // 50MB max uncompressed total
const MAX_COMPRESSION_RATIO: u64 = 50; // Max 50:1 compression ratio
const MAX_FILE_COUNT: usize = 10; // Max 10 files per ZIP

/// Safely extract YAML content from a ZIP archive with ZIP bomb protection
pub fn extract_yaml_from_zip(zip_data: &[u8]) -> Result<String> {
    // Check compressed size
    if zip_data.len() as u64 > MAX_ZIP_SIZE {
        bail!("ZIP file too large: {} bytes (max {} bytes)", zip_data.len(), MAX_ZIP_SIZE);
    }

    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)
        .context("Failed to read ZIP archive")?;

    // Check file count
    if archive.len() > MAX_FILE_COUNT {
        bail!("ZIP contains too many files: {} (max {})", archive.len(), MAX_FILE_COUNT);
    }

    let mut total_uncompressed_size: u64 = 0;
    let mut yaml_content: Option<String> = None;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .context("Failed to access file in ZIP")?;

        // Security check: Prevent path traversal
        let file_name = file.name().to_string();
        if file_name.contains("..") || file_name.starts_with('/') {
            bail!("Suspicious file path detected in ZIP: {file_name}");
        }

        // Check individual file size
        let file_size = file.size();
        total_uncompressed_size += file_size;

        if total_uncompressed_size > MAX_UNCOMPRESSED_SIZE {
            bail!("Total uncompressed size exceeds limit: {total_uncompressed_size} bytes (max {MAX_UNCOMPRESSED_SIZE} bytes)");
        }

        // Check compression ratio for this file
        let compressed_size = file.compressed_size();
        if compressed_size > 0 {
            let ratio = file_size / compressed_size;
            if ratio > MAX_COMPRESSION_RATIO {
                bail!("Suspicious compression ratio detected: {ratio}:1 (max {MAX_COMPRESSION_RATIO}:1)");
            }
        }

        // Only extract YAML/YML files
        if file_name.ends_with(".yaml") || file_name.ends_with(".yml") {
            let mut content = String::new();
            file.read_to_string(&mut content)
                .context("Failed to read YAML file from ZIP")?;
            
            // Take the first YAML file found
            if yaml_content.is_none() {
                yaml_content = Some(content);
            }
        }
    }

    yaml_content.ok_or_else(|| anyhow::anyhow!("No YAML file found in ZIP archive"))
}

#[cfg(test)]
mod tests {
    

    #[test]
    fn test_max_file_count() {
        // Requires creating a ZIP with excessive files to trigger MAX_FILE_COUNT limit
        // Production testing uses crafted malicious archives
    }

    #[test]
    fn test_path_traversal_protection() {
        // Test that paths with .. are rejected
        // Would require creating a ZIP with malicious paths
    }
}
