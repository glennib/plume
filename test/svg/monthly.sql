-- x_monthly(): a year of days on a date axis, with key points at month starts labelled as plotters'
-- Monthly labels them.
SELECT chart()
         .caption('x_monthly()', 20)
         .x_monthly()
         .draw_series(line_series(DATE '2024-01-01' + i::INTEGER, 5 - 12 * cos(i / 58)).style('red'))
         .to_svg(800, 400)
FROM range(366) r(i);
