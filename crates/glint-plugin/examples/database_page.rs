//! Demonstrates the generic host/page contract; no database connection is opened.
use glint_plugin::{HostRegistry, Limits, Page, Plugin, PluginRuntime};
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let plugin=Plugin::from_source(&json!({
        "id":"example.database-page", "name":"Database page example", "version":"1.0.0", "api_version":1,
        "entry":"main.js", "capabilities":["database.metadata.read"], "hooks":["database.page"],
        "contributions":[{"id":"schema","kind":"page","title":"Schema","hook":"database.page"}]
    }).to_string(), r#"
        glint.on('database.page', input => ({
            title: 'Database schema',
            body: {type:'column', children:[
                {type:'text', text:'Host-provided metadata; the plugin holds no connection.'},
                {type:'button', id:'refresh', label:'Refresh', action:'database.page', data:input},
                {type:'table', id:'columns', columns:['Name','Type'], rows:glint.host.call('database.describe',input.table)}
            ]}
        }));
    "#)?;
    let mut host = HostRegistry::default();
    host.register("database.describe", "database.metadata.read", |_| {
        Ok(json!([["id", "integer"], ["title", "text"]]))
    })?;
    let mut runtime = PluginRuntime::activate(
        &plugin,
        host,
        plugin.manifest.capabilities.clone(),
        Limits::default(),
    )?;
    let page: Page =
        serde_json::from_value(runtime.invoke("database.page", &json!({"table":"notes"}))?)?;
    page.validate()?;
    println!("{}", serde_json::to_string_pretty(&page)?);
    Ok(())
}
