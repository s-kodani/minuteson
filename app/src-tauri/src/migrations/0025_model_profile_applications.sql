-- Immutable applied configuration versions; local availability is re-evaluated.
CREATE UNIQUE INDEX model_profile_version ON model_profiles(name,profile_version);
CREATE TRIGGER model_profile_fixed BEFORE UPDATE ON model_profiles
WHEN OLD.state!='preparing' OR NEW.id!=OLD.id OR NEW.name!=OLD.name OR NEW.profile_version!=OLD.profile_version
 OR NEW.state NOT IN ('ready','unavailable')
BEGIN SELECT RAISE(ABORT,'completed model profile is immutable'); END;
CREATE TRIGGER model_profile_no_delete BEFORE DELETE ON model_profiles
BEGIN SELECT RAISE(ABORT,'model profile history is retained'); END;
CREATE TRIGGER profile_component_insert BEFORE INSERT ON profile_components
WHEN NOT EXISTS(SELECT 1 FROM model_profiles WHERE id=NEW.profile_id AND state='preparing')
BEGIN SELECT RAISE(ABORT,'profile components require an unsealed profile'); END;
CREATE TRIGGER profile_component_fixed BEFORE UPDATE ON profile_components
BEGIN SELECT RAISE(ABORT,'model profile components are immutable'); END;
CREATE TRIGGER profile_component_no_delete BEFORE DELETE ON profile_components
BEGIN SELECT RAISE(ABORT,'model profile components are retained'); END;
