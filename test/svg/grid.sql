-- A 2x2 grid (split_evenly) of titled charts under one title; the fourth cell is left blank and
-- shows the grid's root_fill.
SELECT split_evenly([
         (SELECT chart().configure_mesh().x_labels(4).draw()
                   .draw_series(line_series(day, temp, key := city)) FROM weather)
           .titled('Temperature'),
         (SELECT chart().draw_series(histogram_vertical(section, n, order_by := -n).style('blue_400'))
          FROM (SELECT section, count(*) AS n FROM articles GROUP BY section))
           .titled('Articles'),
         (SELECT pie(n, section).label_style(11).percentages(10)
          FROM (SELECT section, count(*) AS n FROM articles GROUP BY section))
           .titled('Share'),
       ], 2, 2)
         .root_fill('grey_100')
         .titled('Overview', 28)
         .to_svg(800, 600);
