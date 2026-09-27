-- Horizontal bars on a category y axis, largest first from the top, with a bar margin and a
-- baseline of 150: counts below it are drawn leftwards from the baseline.
SELECT chart()
         .caption('Articles per section', 20)
         .y_label_area_size(70)
         .configure_mesh().x_desc('articles').x_label_formatter('{:.0f}').draw()
         .draw_series(histogram_horizontal(section, n, order_by := n).style('teal_400').margin(2).baseline(150))
         .to_svg()
FROM (SELECT section, count(*) AS n FROM articles GROUP BY section);
