-- x_yearly(): ten years of monthly timestamps with key points at year starts.
SELECT chart()
         .caption('x_yearly()', 20)
         .x_yearly()
         .draw_series(line_series(TIMESTAMP '2015-01-01' + INTERVAL (m) MONTH, 100 + m + 15 * sin(m / 2)).style('purple', 2))
         .to_svg(800, 400)
FROM range(120) r(m);
