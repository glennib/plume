-- DOUBLE buckets binned by .step(5) into bands from multiples of 5, labelled with the bin starts
-- through a format() template.
SELECT chart()
         .caption('histogram_vertical(x, 1).step(5)', 20)
         .configure_mesh().x_desc('x').y_desc('count').x_label_formatter('{:.0f}').draw()
         .draw_series(histogram_vertical(x, 1).step(5).style('teal_400').margin(1))
         .to_svg()
FROM (SELECT round(50 + 25 * sin(i * 0.37) * cos(i * 0.011) + 8 * sin(i * 1.3), 2) AS x FROM range(400) r(i));
