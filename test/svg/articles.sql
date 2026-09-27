-- Bars on a category axis, in order_by order.
SELECT chart().caption('Articles per section')
         .draw_series(histogram_vertical(section, n, order_by := -n).style('blue_400'))
         .to_svg()
FROM (SELECT section, count(*) AS n FROM articles GROUP BY section);
