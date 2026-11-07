-- The routes a plugin declared in its manifest for people to call through the core (ADR-0078): method,
-- path, who may call it. Checked when the plugin was installed; one that does not read back is left out.
ALTER TABLE plugins.plugin_installs
    ADD COLUMN routes jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(routes) = 'array');
