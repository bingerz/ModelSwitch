//! Data migration framework for schema evolution.
//!
//! As ModelSwitch evolves, data files (virtual_keys.json, audit.ndjson, etc.)
//! may need schema updates. This module provides a framework for detecting
//! old schema versions and automatically migrating them at startup, before
//! the data stores load.
//!
//! ## Schema Versioning Strategy
//!
//! - **JSON store files** (e.g. `virtual_keys.json`): A `schema_version` key
//!   is stored at the top level alongside the HashMap entries. The
//!   `PersistedStore` strips this key during deserialization so it does not
//!   interfere with `HashMap<Uuid, V>` parsing.
//!
//! - **NDJSON files** (e.g. `audit.ndjson`): Each line (entry) includes a
//!   `schema_version` field. serde ignores unknown fields during
//!   deserialization, so this is transparent to `AuditEntry`.
//!
//! ## Adding a New Migration
//!
//! 1. Bump `CURRENT_SCHEMA_VERSION`.
//! 2. Add a `migrate_vN_to_vN1` function.
//! 3. Register it in the `while version < CURRENT_SCHEMA_VERSION` loop in
//!    both [`migrate_json_file`] and [`migrate_ndjson_file`].

use serde_json::Value;
use std::path::Path;
use thiserror::Error;

/// Current schema version for all data files.
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Errors that can occur during data migration.
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("failed to read data file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse JSON in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write migrated data to {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create backup at {path}: {source}")]
    Backup {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Migrate a data file to the current schema version.
///
/// This function:
/// 1. Reads the file (JSON or NDJSON, detected by extension).
/// 2. Checks for a `schema_version` field (default 0 if missing).
/// 3. Runs applicable migrations in sequence.
/// 4. Backs up the original file (`.bak` suffix) before writing.
/// 5. Writes the migrated data back.
///
/// Returns `Ok(())` if the file is already current, was successfully
/// migrated, or does not exist (no-op for missing files).
pub fn migrate_data_file(path: &Path) -> Result<(), MigrationError> {
    if !path.exists() {
        tracing::debug!(path = %path.display(), "skipping migration: file does not exist");
        return Ok(());
    }

    let is_ndjson = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "ndjson")
        .unwrap_or(false);

    let path_str = path.display().to_string();

    if is_ndjson {
        migrate_ndjson_file(path, &path_str)
    } else {
        migrate_json_file(path, &path_str)
    }
}

// ── JSON file migration ─────────────────────────────────────────────────────

/// Migrate a regular JSON file (e.g., `virtual_keys.json`).
fn migrate_json_file(path: &Path, path_str: &str) -> Result<(), MigrationError> {
    let content = std::fs::read_to_string(path).map_err(|e| MigrationError::Read {
        path: path_str.to_string(),
        source: e,
    })?;

    let mut data: Value = serde_json::from_str(&content).map_err(|e| MigrationError::Parse {
        path: path_str.to_string(),
        source: e,
    })?;

    let current_version = get_schema_version(&data);

    if current_version >= CURRENT_SCHEMA_VERSION {
        tracing::debug!(
            path = path_str,
            version = current_version,
            "schema already at current version, skipping migration"
        );
        return Ok(());
    }

    tracing::info!(
        path = path_str,
        from_version = current_version,
        to_version = CURRENT_SCHEMA_VERSION,
        "migrating data file"
    );

    // Back up the original file before migrating.
    let backup_path = format!("{}.bak", path_str);
    std::fs::copy(path, &backup_path).map_err(|e| MigrationError::Backup {
        path: backup_path.clone(),
        source: e,
    })?;
    tracing::info!(backup = %backup_path, "created backup before migration");

    // Run migrations in sequence.
    let mut version = current_version;
    while version < CURRENT_SCHEMA_VERSION {
        match version {
            0 => migrate_v0_to_v1(&mut data),
            _ => {
                tracing::warn!(version, "no migration registered for this version");
                break;
            }
        }
        version += 1;
    }

    set_schema_version(&mut data, CURRENT_SCHEMA_VERSION);

    let migrated_json = serde_json::to_string_pretty(&data).map_err(|e| MigrationError::Parse {
        path: path_str.to_string(),
        source: e,
    })?;

    std::fs::write(path, migrated_json).map_err(|e| MigrationError::Write {
        path: path_str.to_string(),
        source: e,
    })?;

    tracing::info!(
        path = path_str,
        version = CURRENT_SCHEMA_VERSION,
        "migration complete"
    );

    Ok(())
}

// ── NDJSON file migration ───────────────────────────────────────────────────

/// Migrate an NDJSON file (e.g., `audit.ndjson`).
///
/// Each line is parsed and migrated independently. Malformed lines are
/// preserved as-is.
fn migrate_ndjson_file(path: &Path, path_str: &str) -> Result<(), MigrationError> {
    let content = std::fs::read_to_string(path).map_err(|e| MigrationError::Read {
        path: path_str.to_string(),
        source: e,
    })?;

    let mut any_migrated = false;
    let mut migrated_lines: Vec<String> = Vec::with_capacity(content.lines().count());

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            migrated_lines.push(String::new());
            continue;
        }

        let mut entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    path = path_str,
                    error = %e,
                    "skipping malformed NDJSON line during migration"
                );
                migrated_lines.push(line.to_string());
                continue;
            }
        };

        let current_version = get_schema_version(&entry);

        if current_version < CURRENT_SCHEMA_VERSION {
            let mut version = current_version;
            while version < CURRENT_SCHEMA_VERSION {
                match version {
                    0 => migrate_v0_to_v1(&mut entry),
                    _ => break,
                }
                version += 1;
            }
            set_schema_version(&mut entry, CURRENT_SCHEMA_VERSION);
            any_migrated = true;
        }

        migrated_lines.push(serde_json::to_string(&entry).unwrap_or_else(|_| line.to_string()));
    }

    if !any_migrated {
        tracing::debug!(
            path = path_str,
            "NDJSON file already at current schema version"
        );
        return Ok(());
    }

    // Back up the original file.
    let backup_path = format!("{}.bak", path_str);
    std::fs::copy(path, &backup_path).map_err(|e| MigrationError::Backup {
        path: backup_path.clone(),
        source: e,
    })?;
    tracing::info!(backup = %backup_path, "created backup before NDJSON migration");

    let migrated_content = migrated_lines.join("\n") + "\n";
    std::fs::write(path, migrated_content).map_err(|e| MigrationError::Write {
        path: path_str.to_string(),
        source: e,
    })?;

    tracing::info!(
        path = path_str,
        version = CURRENT_SCHEMA_VERSION,
        "NDJSON migration complete"
    );

    Ok(())
}

// ── Schema version helpers ──────────────────────────────────────────────────

/// Extract the schema version from a JSON value. Returns 0 if not present
/// or not a valid number.
fn get_schema_version(data: &Value) -> u32 {
    data.get("schema_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0)
}

/// Set the schema version on a JSON value. No-op if the value is not an
/// object.
fn set_schema_version(data: &mut Value, version: u32) {
    if let Some(obj) = data.as_object_mut() {
        obj.insert(
            "schema_version".to_string(),
            Value::Number(serde_json::Number::from(version)),
        );
    }
}

// ── Migration functions ─────────────────────────────────────────────────────

/// Migration v0 to v1 for JSON data.
///
/// For JSON store files (e.g. `virtual_keys.json`): ensures all object
/// entries that represent VirtualKey records have the `rpm_limit`,
/// `tpm_limit`, `expires_at`, and `group` fields (set to `null` if missing).
/// The `schema_version` key itself is skipped.
///
/// For NDJSON entries (e.g. `audit.ndjson`): ensures each entry has the
/// `prev_hash` field (set to `null` if missing).
pub fn migrate_v0_to_v1(data: &mut Value) {
    // Determine if this looks like a JSON store file (flat object of entries)
    // or a single NDJSON entry.
    let Some(obj) = data.as_object_mut() else {
        return;
    };

    // If the object has many keys and looks like a HashMap store (keys are
    // UUID-like), treat it as a store file. Otherwise, treat it as a single
    // entry (NDJSON line).
    let is_store_file = obj.len() > 1
        && obj
            .keys()
            .filter(|k| k.as_str() != "schema_version")
            .take(2)
            .all(|k| looks_like_uuid(k));

    if is_store_file {
        migrate_v0_to_v1_store(obj);
    } else {
        migrate_v0_to_v1_entry(obj);
    }
}

/// Migrate a JSON store file (HashMap of entries) from v0 to v1.
///
/// Ensures each entry has `rpm_limit`, `tpm_limit`, `expires_at`, and
/// `group` fields.
fn migrate_v0_to_v1_store(obj: &mut serde_json::Map<String, Value>) {
    const REQUIRED_FIELDS: &[&str] = &["rpm_limit", "tpm_limit", "expires_at", "group"];
    let mut migrated_count = 0u32;

    for (key, value) in obj.iter_mut() {
        if key == "schema_version" {
            continue;
        }
        if let Some(entry) = value.as_object_mut() {
            let mut changed = false;
            for field in REQUIRED_FIELDS {
                if !entry.contains_key(*field) {
                    entry.insert((*field).to_string(), Value::Null);
                    changed = true;
                }
            }
            if changed {
                migrated_count += 1;
            }
        }
    }

    if migrated_count > 0 {
        tracing::info!(
            count = migrated_count,
            "v0->v1: added missing fields to store entries"
        );
    }
}

/// Migrate a single NDJSON entry from v0 to v1.
///
/// Ensures the entry has a `prev_hash` field (set to `null` if missing).
fn migrate_v0_to_v1_entry(obj: &mut serde_json::Map<String, Value>) {
    if !obj.contains_key("prev_hash") {
        obj.insert("prev_hash".to_string(), Value::Null);
        tracing::debug!("v0->v1: added prev_hash field to entry");
    }
}

/// Heuristic: check if a string looks like a UUID (contains at least one
/// dash with hex characters). Used to distinguish store files from single
/// entries.
fn looks_like_uuid(s: &str) -> bool {
    s.contains('-') && s.len() >= 32
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Create a temp file with the given content and return its path.
    fn write_temp(content: &str, suffix: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-migration-test-{}-{}.{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple(),
            suffix
        ));
        std::fs::write(&path, content).expect("write temp file");
        path
    }

    // ── JSON store file migration ──────────────────────────────────────────

    #[test]
    fn migrate_json_adds_missing_fields_to_store_entries() {
        let old_data = json!({
            "550e8400-e29b-41d4-a716-446655440000": {
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "key_hash": "abc123",
                "key_prefix": "ms-vk-test",
                "name": "test-key",
                "daily_budget_cents": 100,
                "monthly_budget_cents": 1000,
                "enabled": true,
                "created_at": "2025-01-01T00:00:00Z",
                "spend": {}
            },
            "660e8400-e29b-41d4-a716-446655440001": {
                "id": "660e8400-e29b-41d4-a716-446655440001",
                "key_hash": "def456",
                "key_prefix": "ms-vk-abc",
                "name": "another-key",
                "daily_budget_cents": null,
                "monthly_budget_cents": null,
                "enabled": false,
                "created_at": "2025-06-01T00:00:00Z",
                "spend": {}
            }
        });

        let path = write_temp(&old_data.to_string(), "json");
        let result = migrate_data_file(&path);
        assert!(result.is_ok());

        let migrated: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        // schema_version should be set
        assert_eq!(
            migrated["schema_version"].as_u64(),
            Some(1),
            "schema_version should be 1 after migration"
        );

        // Each entry should have the required fields
        for (_key, entry) in migrated
            .as_object()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.as_str() != "schema_version")
        {
            assert!(
                entry.get("rpm_limit").is_some(),
                "rpm_limit should be present"
            );
            assert!(
                entry.get("tpm_limit").is_some(),
                "tpm_limit should be present"
            );
            assert!(
                entry.get("expires_at").is_some(),
                "expires_at should be present"
            );
            assert!(entry.get("group").is_some(), "group should be present");
        }

        // Backup should exist
        assert!(
            std::path::Path::new(&format!("{}.bak", path.display())).exists(),
            "backup file should exist"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.bak", path.display()));
    }

    #[test]
    fn migrate_json_skips_already_migrated_file() {
        let data = json!({
            "schema_version": 1,
            "550e8400-e29b-41d4-a716-446655440000": {
                "rpm_limit": null,
                "tpm_limit": null,
                "expires_at": null,
                "group": null
            }
        });

        let path = write_temp(&data.to_string(), "json");
        let result = migrate_data_file(&path);
        assert!(result.is_ok());

        // File content should be unchanged (no backup created)
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_u64(), Some(1));

        assert!(
            !std::path::Path::new(&format!("{}.bak", path.display())).exists(),
            "no backup should be created when already at current version"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migrate_json_missing_file_is_noop() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-migration-nonexistent-{}.json",
            uuid::Uuid::new_v4().simple()
        ));
        let result = migrate_data_file(&path);
        assert!(result.is_ok(), "missing file should return Ok(())");
    }

    // ── NDJSON migration ───────────────────────────────────────────────────

    #[test]
    fn migrate_ndjson_adds_prev_hash_and_schema_version() {
        let old_content = json!({
            "timestamp": "2025-01-01T00:00:00Z",
            "action": "channel.create",
            "actor": "127.0.0.1",
            "target": "ch-001",
            "details": {}
        })
        .to_string();

        let ndjson = format!("{old_content}\n{old_content}\n");
        let path = write_temp(&ndjson, "ndjson");

        let result = migrate_data_file(&path);
        assert!(result.is_ok());

        let migrated = std::fs::read_to_string(&path).unwrap();
        for line in migrated.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: Value = serde_json::from_str(trimmed).unwrap();
            assert_eq!(entry["schema_version"].as_u64(), Some(1));
            assert!(
                entry.get("prev_hash").is_some(),
                "prev_hash should be present after migration"
            );
        }

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.bak", path.display()));
    }

    #[test]
    fn migrate_ndjson_skips_already_migrated() {
        let entry = json!({
            "timestamp": "2025-01-01T00:00:00Z",
            "action": "test",
            "actor": "system",
            "target": "t1",
            "details": {},
            "prev_hash": "abc",
            "schema_version": 1
        })
        .to_string();

        let ndjson = format!("{entry}\n");
        let path = write_temp(&ndjson, "ndjson");

        let result = migrate_data_file(&path);
        assert!(result.is_ok());

        // No backup should be created
        assert!(
            !std::path::Path::new(&format!("{}.bak", path.display())).exists(),
            "no backup for already-migrated NDJSON"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migrate_ndjson_preserves_malformed_lines() {
        let ndjson = "{\"valid\":\"yes\"}\nthis is not json\n";
        let path = write_temp(ndjson, "ndjson");

        let result = migrate_data_file(&path);
        assert!(
            result.is_ok(),
            "migration should not fail on malformed lines"
        );

        let migrated = std::fs::read_to_string(&path).unwrap();
        assert!(
            migrated.contains("this is not json"),
            "malformed line should be preserved"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.bak", path.display()));
    }

    // ── Unit tests for migration functions ─────────────────────────────────

    #[test]
    fn migrate_v0_to_v1_adds_fields_to_store_entries() {
        let mut data = json!({
            "550e8400-e29b-41d4-a716-446655440000": {
                "name": "key1"
            },
            "660e8400-e29b-41d4-a716-446655440001": {
                "name": "key2",
                "rpm_limit": 100
            }
        });

        migrate_v0_to_v1(&mut data);

        let entries = data.as_object().unwrap();
        for (_key, entry) in entries.iter() {
            let entry_obj = entry.as_object().unwrap();
            assert!(entry_obj.contains_key("rpm_limit"));
            assert!(entry_obj.contains_key("tpm_limit"));
            assert!(entry_obj.contains_key("expires_at"));
            assert!(entry_obj.contains_key("group"));
        }

        // Existing values should be preserved
        assert_eq!(
            data["660e8400-e29b-41d4-a716-446655440001"]["rpm_limit"].as_u64(),
            Some(100)
        );
    }

    #[test]
    fn migrate_v0_to_v1_adds_prev_hash_to_entry() {
        let mut data = json!({
            "timestamp": "2025-01-01T00:00:00Z",
            "action": "test",
            "actor": "system",
            "target": "t1",
            "details": {}
        });

        migrate_v0_to_v1(&mut data);

        assert!(data.get("prev_hash").is_some(), "prev_hash should be added");
    }

    #[test]
    fn migrate_v0_to_v1_preserves_existing_prev_hash() {
        let mut data = json!({
            "timestamp": "2025-01-01T00:00:00Z",
            "action": "test",
            "actor": "system",
            "target": "t1",
            "details": {},
            "prev_hash": "existing-hash"
        });

        migrate_v0_to_v1(&mut data);

        assert_eq!(data["prev_hash"].as_str(), Some("existing-hash"));
    }

    #[test]
    fn get_schema_version_defaults_to_zero() {
        let data = json!({"key": "value"});
        assert_eq!(get_schema_version(&data), 0);
    }

    #[test]
    fn get_schema_version_reads_existing() {
        let data = json!({"schema_version": 1, "key": "value"});
        assert_eq!(get_schema_version(&data), 1);
    }

    #[test]
    fn set_schema_version_updates_value() {
        let mut data = json!({"key": "value"});
        set_schema_version(&mut data, 1);
        assert_eq!(data["schema_version"].as_u64(), Some(1));
    }
}
