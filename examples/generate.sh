#!/usr/bin/env bash
# Regenerate all example outputs.
# Run from the project root after installing: cargo install --path .
set -euo pipefail

KENNEDY=examples/kennedy/kennedy.ged

echo "==> Kennedy: simple text (5 generations from patriarch Patrick Kennedy)"
genechart "$KENNEDY" --root I46 --gen 5 --text \
  --pref 'output.text.title = "The Kennedy Family"' \
  -o examples/kennedy/kennedy_simple.txt

echo "==> Kennedy: boxed_couples SVG (4 generations from Joseph Kennedy Sr.)"
genechart "$KENNEDY" --root I1 --gen 4 --type boxed_couples --svg \
  -o examples/kennedy/kennedy_boxed_couples.svg

echo "==> Kennedy: ancestor fan SVG (JFK, 4 generations)"
genechart "$KENNEDY" --root I0 --dir ancestors --type fan --gen 4 --svg \
  -o examples/kennedy/kennedy_fan.svg

echo "==> Kennedy: fancy SVG (3 generations from Joseph Kennedy Sr.)"
genechart "$KENNEDY" --root I1 --gen 3 --type fancy --svg \
  -o examples/kennedy/kennedy_fancy.svg

echo "Done. Outputs written to examples/kennedy/"

SHAKESPEARE=examples/shakespeare/shakespeare.ged

echo "==> Shakespeare: simple text (4 generations from patriarch John Shakespeare)"
genechart "$SHAKESPEARE" --root I0003 --gen 4 --text \
  --pref 'output.text.title = "The Shakespeare Family"' \
  -o examples/shakespeare/shakespeare_simple.txt

echo "==> Shakespeare: boxed_couples SVG (3 generations from William Shakespeare)"
genechart "$SHAKESPEARE" --root I0001 --gen 3 --type boxed_couples --svg \
  -o examples/shakespeare/shakespeare_boxed_couples.svg

echo "==> Shakespeare: ancestor fan SVG (William Shakespeare, 4 generations)"
genechart "$SHAKESPEARE" --root I0001 --dir ancestors --type fan --gen 3 --svg \
  -o examples/shakespeare/shakespeare_fan.svg

echo "==> Shakespeare: fancy SVG (3 generations from William Shakespeare)"
genechart "$SHAKESPEARE" --root I0001 --gen 3 --type fancy --svg \
  -o examples/shakespeare/shakespeare_fancy.svg

echo "Done. Outputs written to examples/shakespeare/"
