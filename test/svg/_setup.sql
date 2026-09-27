-- Shared data for the SVG snapshots.
CREATE TABLE weather AS
SELECT city, DATE '2024-01-01' + d::INTEGER AS day,
       round(-2 + 4 * c + 0.3 * d + 5 * sin(d / 5 + c), 1) AS temp
FROM (VALUES ('Oslo', 0), ('Bergen', 1), ('Tromsø', 2)) cities(city, c), range(60) r(d);

CREATE TABLE articles AS
SELECT section
FROM (VALUES ('sport', 412), ('culture', 158), ('news', 530), ('economy', 201), ('opinion', 97)) s(section, n),
     range(n);
