-- The README's first example and its picture: the query verbatim, with to_svg() in place of
-- show().
SELECT chart()
         .caption('Oslo temperature, 2024', 30)
         .configure_mesh().x_desc('day').y_desc('°C').x_label_formatter('%b').draw()
         .draw_series(line_series(day, temp).style('red'))
         .to_svg()
FROM 'examples/weather.csv'
WHERE city = 'Oslo';
