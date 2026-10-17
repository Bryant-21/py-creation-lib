use pyo3::prelude::*;

pub fn with_context<T>(
    context: &Bound<'_, PyAny>,
    body: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
) -> PyResult<T> {
    let py = context.py();
    let value = context.call_method0("__enter__")?;
    match body(&value) {
        Ok(result) => {
            context.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
            Ok(result)
        }
        Err(error) => {
            context.call_method1(
                "__exit__",
                (error.get_type(py), error.value(py), error.traceback(py)),
            )?;
            Err(error)
        }
    }
}

pub fn with_database<T>(
    factory: &Bound<'_, PyAny>,
    body: impl FnOnce(&Bound<'_, PyAny>, &crate::database::Database) -> PyResult<T>,
) -> PyResult<T> {
    with_context(&factory.call0()?, |wrapper| {
        let native = wrapper.getattr("native")?;
        let native = native.extract::<PyRef<'_, crate::database::Database>>()?;
        body(wrapper, &native)
    })
}
