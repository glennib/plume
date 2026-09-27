-- boxplot_horizontal(bucket, value) in order_by order (the first bucket at the bottom), in the
-- default palette colour.
SELECT chart()
         .caption('boxplot_horizontal', 20)
         .y_label_area_size(50)
         .draw_series(boxplot_horizontal(g, v, order_by := list_position(['west', 'east', 'south', 'north'], g)).width(20))
         .to_svg()
FROM (SELECT g, 10 + 3 * gi + ((i * 37 + gi * 11) % 40) / 4 AS v
      FROM (VALUES ('north', 0), ('south', 1), ('east', 2), ('west', 3)) t(g, gi), range(40) r(i)
      UNION ALL SELECT 'east', 45);
