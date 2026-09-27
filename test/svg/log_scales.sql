-- Log scales on both axes: a power law is a straight line, and the labels are plotters' float
-- printer with scientific notation (1, 10, 1e6).
SELECT chart()
         .caption('x² and x³ on log axes', 20)
         .x_log_scale()
         .y_log_scale()
         .configure_mesh().x_desc('x').draw()
         .draw_series(line_series(x, x * x).label('x²'))
         .draw_series(point_series(x, x * x * x).marker('triangle').size(4).label('x³'))
         .configure_series_labels().position('upper_left').border_style('black').draw()
         .to_svg()
FROM (SELECT pow(10, i / 10.0) AS x FROM range(0, 31) r(i));
