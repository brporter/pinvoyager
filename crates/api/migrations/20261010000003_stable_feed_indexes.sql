CREATE INDEX voyages_public_page_idx ON voyages (published_at DESC, id DESC) WHERE published;
CREATE INDEX voyages_owner_public_page_idx ON voyages (owner_id, published_at DESC, id DESC) WHERE published;
