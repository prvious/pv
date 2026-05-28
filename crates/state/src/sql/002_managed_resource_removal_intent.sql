ALTER TABLE managed_resource_tracks
ADD COLUMN removal_prune INTEGER NOT NULL DEFAULT 0
CHECK (removal_prune IN (0, 1));

ALTER TABLE managed_resource_tracks
ADD COLUMN removal_force INTEGER NOT NULL DEFAULT 0
CHECK (removal_force IN (0, 1));
