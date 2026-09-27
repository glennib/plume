-- A timestamp axis with a line with markers and points, on a grey background.
SELECT chart()
         .root_fill('grey_100')
         .draw_series(line_series(ts, v).label('load').point_size(2))
         .draw_series(point_series(ts, v).label('samples'))
         .to_svg()
FROM (SELECT TIMESTAMP '2024-01-01' + h * INTERVAL 1 hour AS ts, sin(h / 6) * 10 + 20 AS v FROM range(48) r(h));
