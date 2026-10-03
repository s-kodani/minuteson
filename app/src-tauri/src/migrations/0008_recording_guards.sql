CREATE UNIQUE INDEX recording_sessions_one_active
ON recording_sessions((1))
WHERE state IN ('preparing', 'recording', 'stopping');
