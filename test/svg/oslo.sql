-- The plan's first worked example: caption, builder sizes, y range, mesh descriptions, a styled
-- and labelled line on a date axis, and a legend with a border.
SELECT chart()
         .caption('Oslo temperature', 30)
         .margin(10)
         .x_label_area_size(30)
         .y_label_area_size(40)
         .y_range(-10, 30)
         .configure_mesh().x_desc('day').y_desc('°C').draw()
         .draw_series(line_series(day, temp).style('red').label('temp'))
         .configure_series_labels().border_style('black').draw()
         .to_svg(800, 600)
FROM weather
WHERE city = 'Oslo';
