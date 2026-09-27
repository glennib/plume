-- Secondary axes: precipitation as bars on the secondary y axis (right), temperature as a line on
-- the primary y axis, one legend for both. The bars are drawn first so the line is on top.
SELECT chart().caption('Bergen climate', 24)
         .y_range(0, 16)
         .configure_mesh().y_desc('°C').draw()
         .configure_secondary_axes().y_desc('mm').y_label_formatter('{:.0f}').draw()
         .draw_secondary_series(histogram_vertical(month, rain, order_by := m)
                                  .style(mix('blue', 0.4)).label('precipitation'))
         .draw_series(line_series(month, temp, order_by := m).style('red', 2).point_size(3)
                        .label('temperature'))
         .configure_series_labels().position('upper_left').background_style('white')
             .border_style('black').draw()
         .to_svg()
FROM (VALUES (1, 'Jan', 2.1, 250), (2, 'Feb', 2.2, 191), (3, 'Mar', 3.9, 211),
             (4, 'Apr', 6.9, 150), (5, 'May', 10.8, 118), (6, 'Jun', 13.8, 127),
             (7, 'Jul', 15.2, 162), (8, 'Aug', 15.1, 238), (9, 'Sep', 12.1, 293),
             (10, 'Oct', 8.4, 288), (11, 'Nov', 4.9, 283), (12, 'Dec', 2.7, 274))
     t(m, month, temp, rain);
