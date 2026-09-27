-- Every kind of MESH setting: a strftime formatter on the date x axis and a format() template
-- on the numeric y axis, label and description fonts, light, bold and axis line styles, the
-- vertical grid lines disabled, tick sizes and label offsets.
SELECT chart()
         .caption('Oslo, styled mesh', font('serif', 24, 'bold'))
         .x_label_area_size(50).y_label_area_size(80)
         .configure_mesh()
           .x_desc('day').y_desc('temperature')
           .x_label_formatter('%b %d')
           .y_label_formatter('{:+.1f} °C')
           .x_labels(6).y_labels(8)
           .label_style(font('sans-serif', 11).color('grey_700'))
           .axis_desc_style(font('serif', 14, 'italic'))
           .light_line_style('blue_50')
           .bold_line_style('blue_200', 1)
           .axis_style('blue_900', 2)
           .disable_x_mesh()
           .set_tick_mark_size('bottom', 8)
           .set_tick_mark_size('left', 3)
           .x_label_offset(4)
           .y_label_offset(-3)
           .draw()
         .draw_series(line_series(day, temp).style('red', 2))
         .to_svg(800, 500)
FROM weather
WHERE city = 'Oslo';
