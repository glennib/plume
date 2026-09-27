-- A reversed x axis and a y range narrower than the data, whose points are clamped.
SELECT chart().x_range(20, 0).y_range(0, 50).draw_series(line_series(i, (i - 10) ** 2)).to_svg()
FROM range(20) r(i);
