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

baseline="$(jq -er '.line_coverage_percent | numbers' "${baseline_path}")"
actual="$(jq -er '
  [ .data[]?.totals.lines? | select(.count != null and .count > 0) ] as $totals
  | if ($totals | length) == 0 then error("no line coverage totals")
    else ([ $totals[] | .covered ] | add) / ([ $totals[] | .count ] | add) * 100
    end
' "${report_path}")"

printf 'Protocol line coverage: %.4f%% (minimum: %.4f%%)\n' "${actual}" "${baseline}"
awk -v actual="${actual}" -v baseline="${baseline}" 'BEGIN { exit actual + 0.0001 < baseline }' || {
  echo "coverage regression: ${actual}% is below ${baseline}%" >&2
  exit 1
}
