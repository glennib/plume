-- dashed_line_series next to a line_series: the default dash (5 px, 5 px apart) with a stroke width
-- of 2, and a long dash. The CASE expressions pick one city per series, since rows with a NULL are
-- skipped.
SELECT chart()
         .caption('dashed_line_series', 20)
         .draw_series(line_series(day, CASE WHEN city = 'Oslo' THEN temp END).label('Oslo'))
         .draw_series(dashed_line_series(day, CASE WHEN city = 'Bergen' THEN temp END).stroke_width(2).label('Bergen'))
         .draw_series(dashed_line_series(day, CASE WHEN city = 'Tromsø' THEN temp END).size(12).spacing(4).label('Tromsø'))
         .configure_series_labels().position('upper_left').draw()
         .to_svg()
FROM weather;
