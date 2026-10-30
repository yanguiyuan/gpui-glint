@{
    api = @{
        Package = 'glint-api'
        Name = 'Glint API'
        Id = '7D39F166-C38C-45EA-B253-053B92032909'
        ModelId = 'dev.glint.api'
        FilePrefix = 'Glint-API'
        BuildDirectory = 'windows-installer'
        Icon = 'crates/glint-api/assets/glint-api-icon.ico'
    }
    workbench = @{
        Package = 'glint-workbench'
        Name = 'Glint Workbench'
        Id = '7B7931DA-05B8-4E6B-9FD4-F33D82DD4B23'
        ModelId = 'dev.glint.workbench'
        FilePrefix = 'Glint-Workbench'
        BuildDirectory = 'windows-workbench-installer'
        Icon = 'crates/glint-api/assets/glint-api-icon.ico'
    }
}
