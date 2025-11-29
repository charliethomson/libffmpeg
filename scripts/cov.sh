
# TODO: install tarpaulin if not installed

COVROOT=".cov"
mkdir -p "$COVROOT"

cargo tarpaulin -o Html -o Lcov --output-dir "$COVROOT"

open "$COVROOT/tarpaulin-report.html"
