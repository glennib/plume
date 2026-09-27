-- A nested grid: one chart on the left, a 2x1 grid of two charts on the right.
SELECT split_evenly([
         (SELECT chart().caption('All cities', 16).configure_mesh().x_labels(4).draw()
                   .draw_series(line_series(day, temp, key := city))
          FROM weather),
         split_evenly([
           (SELECT chart().caption('Oslo', 16).configure_mesh().x_labels(4).draw()
                     .draw_series(line_series(day, temp).style('red'))
            FROM weather WHERE city = 'Oslo'),
           (SELECT chart().caption('Tromsø', 16).configure_mesh().x_labels(4).draw()
                     .draw_series(line_series(day, temp).style('blue'))
            FROM weather WHERE city = 'Tromsø'),
         ], 2, 1),
       ], 1, 2)
         .to_svg(800, 480);
