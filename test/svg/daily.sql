-- Date buckets, one band per day, with a line through the band centres, and a legend at a
-- coordinate.
SELECT chart()
         .draw_series(histogram_vertical(day, v).label('count'))
         .draw_series(line_series(day, v).style('black', 2).label('trend'))
         .configure_series_labels().position(20, 20).border_style('grey', 1).draw()
         .to_svg()
FROM (SELECT DATE '2024-01-01' + d::INTEGER AS day, (d * 37) % 11 AS v FROM range(14) r(d));
