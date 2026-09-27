-- Two series kinds, explicit styles, and a legend position and background.
SELECT chart()
         .configure_mesh().x_desc('x').y_desc('y').draw()
         .draw_series(line_series(i, i * i).style('red').label('y = x²'))
         .draw_series(point_series(i, i * i).style('black').size(4).filled())
         .configure_series_labels().position('upper_left').background_style(mix('white', 0.8)).draw()
         .to_svg(800, 600)
FROM range(10) t(i);
