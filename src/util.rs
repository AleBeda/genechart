/// Check if `input` matches the beginning of `canonical`.
pub fn matches_direction(input: &str, canonical: &str) -> bool {
    !input.is_empty() && canonical.starts_with(input)
}
