-- One line per key, coloured from Palette99 and labelled with the key, with the automatic legend.
SELECT chart().draw_series(line_series(day, temp, key := city)).to_svg() FROM weather;
