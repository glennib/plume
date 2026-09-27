-- Every ChartBuilder size: per-side margins, label areas on all four sides (plotters repeats
-- the labels and descriptions in the top and right areas), then the left and bottom areas set
-- together.
SELECT chart()
         .caption('ChartBuilder sizes', 20)
         .margin_top(5).margin_bottom(20).margin_left(30).margin_right(15)
         .set_all_label_area_size(35)
         .top_x_label_area_size(45)
         .right_y_label_area_size(65)
         .set_left_and_bottom_label_area_size(50)
         .configure_mesh().x_desc('x').y_desc('y').draw()
         .draw_series(line_series(i, i * i).style('purple', 2))
         .to_svg()
FROM range(11) r(i);
