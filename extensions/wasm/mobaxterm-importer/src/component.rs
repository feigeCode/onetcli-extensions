wit_bindgen::generate!({
    path: "../../../wit",
    world: "connection-importer",
});

use onet::extension::{connection_import::CandidateFile, connection_import_host};

struct MobaxtermImporter;

impl Guest for MobaxtermImporter {
    fn descriptor() -> String {
        serde_json::json!({
            "id": "mobaxterm",
            "display_name": "MobaXterm",
            "description": "Import SSH sessions from MobaXterm.ini or .mxtsessions files",
            "icon": "terminal",
            "vendor": "Navop",
            "supported_platforms": ["macos", "windows", "linux"],
            "output_kinds": ["ssh"],
            "capabilities": {
                "supports_scan": true,
                "supports_password_import": false,
                "supports_manual_file_pick": true,
                "supports_incremental_preview": false
            }
        })
        .to_string()
    }

    fn scan() -> String {
        let candidates = connection_import_host::list_candidate_files("mobaxterm");
        let sessions = read_sessions(&candidates);
        let availability = if sessions.is_empty() {
            serde_json::json!("no_data")
        } else {
            serde_json::json!({ "available": { "estimated_count": null } })
        };

        serde_json::json!({
            "importer_id": "mobaxterm",
            "availability": availability,
            "discovered_files": sessions.iter().map(|(path, _)| {
                serde_json::json!({
                    "candidate_id": path,
                    "display_path": path
                })
            }).collect::<Vec<_>>(),
            "warnings": []
        })
        .to_string()
    }

    fn preview(options: ImportOptions) -> String {
        let candidates = connection_import_host::list_candidate_files("mobaxterm");
        let sessions = read_sessions(&candidates);
        let records = crate::mobaxterm::preview_records_from_sources(
            sessions
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.as_slice())),
            options.include_passwords,
        );
        serde_json::to_string(&records).unwrap_or_else(|_| "[]".to_string())
    }
}

fn read_sessions(candidates: &[CandidateFile]) -> Vec<(String, Vec<u8>)> {
    let mut sessions = Vec::new();
    for candidate in candidates {
        if crate::mobaxterm::is_supported_source_path(&candidate.path)
            && let Ok(bytes) = connection_import_host::read_file(&candidate.id)
        {
            sessions.push((candidate.path.clone(), bytes));
        }
    }
    sessions
}

export!(MobaxtermImporter);
