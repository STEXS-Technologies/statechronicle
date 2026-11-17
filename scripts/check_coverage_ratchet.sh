#!/usr/bin/env bash
set -euo pipefail

report_path="${1:-target/coverage.json}"
baseline_path="${2:-ci/coverage-baseline.json}"

command -v jq >/dev/null 2>&1 || {
  echo "jq is required to validate the coverage report" >&2
  exit 1
}
[[ -f "${report_path}" ]] || { echo "coverage report not found: ${report_path}" >&2; exit 1; }
[[ -f "${baseline_path}" ]] || { echo "coverage baseline not found: ${baseline_path}" >&2; exit 1; }

baseline="$(jq -er '.line_coverage_percent | numbers | select(. >= 0 and . <= 100)' "${baseline_path}")"
actual="$(jq -er '
  def valid_lines:
    type == "object"
    and (.count | type) == "number"
    and (.covered | type) == "number"
    and (.count | isfinite) and (.covered | isfinite)
    and .count >= 0 and .covered >= 0 and .covered <= .count
    and .count == (.count | floor) and .covered == (.covered | floor);
  if (.data | type) != "array" or (.data | length) == 0
  then error("no coverage data array")
  elif any(.data[]; (.totals.lines | valid_lines | not))
  then error("invalid line coverage totals")
  else [.data[].totals.lines] as $totals
    | ([ $totals[] | .count ] | add) as $count
    | if $count <= 0 then error("no measured lines")
      else ([ $totals[] | .covered ] | add) / $count * 100
      end
  end
' "${report_path}")"

printf 'Protocol line coverage: %.4f%% (minimum: %.4f%%)\n' "${actual}" "${baseline}"
awk -v actual="${actual}" -v baseline="${baseline}" 'BEGIN { exit actual + 0.0001 < baseline }' || {
  echo "coverage regression: ${actual}% is below ${baseline}%" >&2
  exit 1
}
