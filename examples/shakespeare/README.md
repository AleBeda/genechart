# Shakespeare Family Example

## Source and attribution

`shakespeare.ged` is copied from the [D-Jeffrey/gedcom-samples](https://github.com/D-Jeffrey/gedcom-samples)
repository. Included for educational and demonstration purposes. All genealogical data is
historical and publicly known.

## Scope

31 individuals, 8 families, spanning 4 generations from John Shakespeare (father of the
playwright) through William's grandchildren.

Notable IDs used in the examples:

| ID     | Person                  |
|--------|-------------------------|
| I0001  | William Shakespeare     |
| I0002  | Mary Arden              |
| I0003  | John Shakespeare        |
| I0004  | Anne Hathaway           |
| I0005  | Susanna Shakespeare     |

## Auto-loaded preferences

`shakespeare.toml` has the same basename as `shakespeare.ged`, so genechart loads it
automatically. It sets the default root (William Shakespeare), generation depth, title,
fonts, and highlights file.

## Output files

### `shakespeare_simple.txt` — plain text, 4 generations from patriarch

```
genechart shakespeare.ged --root I0003 --gen 4 --text
```

Full descendant tree starting from John Shakespeare (William's father). Shows all of
William's siblings alongside his own line down to the grandchildren.

### `shakespeare_boxed_couples.svg` — boxed couples, 3 generations from William

```
genechart shakespeare.ged --root I0001 --gen 3 --type boxed_couples --svg
```

Each couple is rendered in a paired box with Unicode box-drawing borders. Spouse names and
life dates appear side by side; children branch downward. William Shakespeare and Anne
Hathaway are highlighted.

### `shakespeare_fan.svg` — ancestor fan, 3 generations from William

```
genechart shakespeare.ged --root I0001 --dir ancestors --type fan --gen 3 --svg
```

Half-circle pedigree fan: William at the centre, parents in the inner ring, and grandparents
at the outer rim. Direction is `ancestors`.

### `shakespeare_fancy.svg` — fancy descendants, 3 generations from William

```
genechart shakespeare.ged --root I0001 --gen 3 --type fancy --svg
```

Cascading descendants layout (SVG/PDF only). Each generation is offset diagonally. William
Shakespeare and Susanna are visually highlighted.

## Highlights

`shakespeare_highlights.txt` marks three individuals for visual emphasis:

```
I0001  William Shakespeare
I0004  Anne Hathaway
I0005  Susanna Shakespeare
```
