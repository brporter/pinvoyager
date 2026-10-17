-- New voyages use approximately 1 km precision when no value is supplied.
ALTER TABLE voyages ALTER COLUMN granularity SET DEFAULT 'km1';
