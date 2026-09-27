-- error_bar_horizontal(y, min, avg, max) on a category y axis, with a stroke width and wide marks.
SELECT chart()
         .caption('error_bar_horizontal', 20)
         .y_label_area_size(60)
         .x_range(0, 8)
         .draw_series(error_bar_horizontal(k, lo, m, hi).style('blue_700', 2).width(14).label('estimate'))
         .to_svg()
FROM (VALUES ('alpha', 2, 3.5, 4), ('beta', 1, 2, 2.5), ('gamma', 3, 5, 7.5), ('delta', 0.5, 1.5, 4)) t(k, lo, m, hi);
