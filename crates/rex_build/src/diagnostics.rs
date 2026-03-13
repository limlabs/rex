use rolldown_error::BatchedBuildDiagnostic;

/// Format build diagnostics into human-readable ariadne output.
pub(crate) fn format_build_diagnostics(diagnostics: &BatchedBuildDiagnostic) -> String {
    diagnostics
        .iter()
        .map(|d| d.to_diagnostic().convert_to_string(false))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Check if all diagnostics are MissingExport (shimmed, non-fatal).
pub(crate) fn is_all_missing_exports(diagnostics: &BatchedBuildDiagnostic) -> bool {
    diagnostics
        .iter()
        .all(|d| d.kind().to_string() == "MISSING_EXPORT")
}
