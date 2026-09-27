-- titled on a single chart: DrawingArea::titled draws the title across the top and the chart
-- below it, here with a FONT.
SELECT chart().draw_series(area_series(day, temp).style(mix('teal', 0.4)).border_style('teal', 2))
         .titled('Oslo, first 60 days of 2024', font('serif', 24, 'bold').color('#333333'))
         .to_svg()
FROM weather
WHERE city = 'Oslo';
