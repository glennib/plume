-- boxplot_vertical(bucket, value): one box per category with plotters' Quartiles (whiskers at the
-- fences, 1.5 IQR out); the outlier of 45 in 'east' is not drawn.
SELECT chart()
         .caption('boxplot_vertical', 20)
         .draw_series(boxplot_vertical(g, v).style('indigo', 2).width(30).label('spread'))
         .to_svg()
FROM (SELECT g, 10 + 3 * gi + ((i * 37 + gi * 11) % 40) / 4 AS v
      FROM (VALUES ('north', 0), ('south', 1), ('east', 2), ('west', 3)) t(g, gi), range(40) r(i)
      UNION ALL SELECT 'east', 45);
