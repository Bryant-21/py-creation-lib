use crate::{
    controller::{callback, submit_call},
    invalid,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

#[pyfunction]
fn controller_category_action(
    manager: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<()> {
    let py = manager.py();
    let library = manager.getattr("library")?;
    let profile = manager.getattr("profile_id")?;
    if !library.is_truthy()? {
        return Err(invalid("Open a local library first."));
    }
    let module = py.import("creation_lib.mod_manager.categories")?;
    let mut cancel = true;
    let mut progress = false;
    let (label, operation, inputs) = match action {
        "import_nexus_categories" => {
            let source = library.clone().unbind();
            let client = manager.getattr("nexus_client")?.unbind();
            let game = manager.getattr("nexus_game")?.unbind();
            let refresh = args.get_item(0)?.is_truthy()?;
            let work = callback(py, move |py, _, kwargs| {
                let module = py.import("creation_lib.mod_manager.categories")?;
                module.getattr("nexus_catalog")?.call(
                    (
                        source.bind(py),
                        client.bind(py),
                        game.bind(py),
                        refresh,
                        true,
                    ),
                    kwargs,
                )?;
                Ok(module
                    .call_method1("inventory", (source.bind(py),))?
                    .unbind())
            })?;
            (
                "Importing Nexus category catalog",
                work.into_bound(py),
                vec![],
            )
        }
        "inspect_categories" => {
            cancel = false;
            (
                "Reading library category taxonomy",
                module.getattr("inventory")?,
                vec![library.clone()],
            )
        }
        "edit_category" => {
            let source = library.clone().unbind();
            let expected = manager.getattr("category_token")?.unbind();
            let values = args.clone().unbind();
            let work = callback(py, move |py, _, kwargs| {
                let module = py.import("creation_lib.mod_manager.categories")?;
                let library = source.bind(py);
                let args = values.bind(py);
                match args.get_item(0)?.extract::<String>()?.as_str() {
                    "save" => module
                        .getattr("save")?
                        .call((library, args.get_item(1)?, args.get_item(3)?), kwargs)?,
                    "rename" => module.getattr("rename")?.call(
                        (
                            library,
                            args.get_item(1)?,
                            args.get_item(2)?,
                            expected.bind(py),
                        ),
                        kwargs,
                    )?,
                    "remove" => module
                        .getattr("remove")?
                        .call((library, args.get_item(1)?, expected.bind(py)), kwargs)?,
                    _ => return Err(invalid("Unsupported category action.")),
                };
                Ok(module.call_method1("inventory", (library,))?.unbind())
            })?;
            (
                "Updating local category taxonomy",
                work.into_bound(py),
                vec![],
            )
        }
        "assign_category" => (
            "Assigning shared mod categories",
            module.getattr("assign")?,
            vec![
                library.clone(),
                profile.clone(),
                args.get_item(0)?,
                args.get_item(1)?,
            ],
        ),
        "toggle_owned_hidden" => {
            let index = manager.getattr("file_index")?;
            let provider = args.get_item(0)?;
            let row = manager
                .getattr("rows_by_id")?
                .call_method1("get", (provider.getattr("mod_id")?,))?;
            if !index.is_truthy()?
                || !index.getattr("profile_id")?.eq(&profile)?
                || !row.is_truthy()?
                || row.getattr("kind")?.extract::<String>()? != "owned"
            {
                return Err(invalid(
                    "Refresh and select a file from a manager-owned mod first.",
                ));
            }
            if !index.getattr("signature")?.eq(py
                .import("creation_lib.mod_manager.conflicts")?
                .call_method1(
                    "rows_signature",
                    (library.call_method1("rows", (&profile,))?,),
                )?)?
            {
                return Err(invalid(
                    "Profile changed after file inspection. Refresh before changing hidden content.",
                ));
            }
            progress = true;
            (
                "Changing owned hidden content with full backup",
                py.import("creation_lib.mod_manager.file_actions")?
                    .getattr(if args.get_item(1)?.is_truthy()? {
                        "toggle_parent_hidden"
                    } else {
                        "toggle_hidden"
                    })?,
                vec![library.clone(), provider],
            )
        }
        _ => return Err(invalid("Unknown category controller operation.")),
    };
    let owner = manager.clone().unbind();
    let action = action.to_string();
    let complete = callback(py, move |py, args, _| {
        let manager = owner.bind(py);
        let result = args.get_item(0)?;
        if action != "inspect_categories" {
            manager.call_method0("refresh")?;
        }
        if [
            "inspect_categories",
            "edit_category",
            "import_nexus_categories",
        ]
        .contains(&action.as_str())
        {
            manager.setattr("categories", result.get_item(0)?)?;
            manager.setattr("category_token", result.get_item(1)?)?;
        }
        let status = match action.as_str() {
            "import_nexus_categories" => "Imported the Nexus category hierarchy; local categories and assignments were preserved.".into(),
            "inspect_categories" => "Read local categories and shared mod assignments.".into(),
            "edit_category" => {
                "Saved local taxonomy. Mod category names are shared across local profiles.".into()
            }
            "assign_category" => "Updated shared mod categories across local profiles.".into(),
            _ => format!(
                "Saved hidden state as {}. Full previous owned content remains in recovery history. Refresh Files to inspect the result.",
                result.str()?
            ),
        };
        manager.setattr("status", status)?;
        Ok(py.None())
    })?;
    submit_call(
        manager,
        label,
        &operation,
        &PyTuple::new(py, inputs)?,
        &PyDict::new(py),
        cancel,
        progress,
        complete.bind(py),
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(controller_category_action, m)?)?;
    Ok(())
}
