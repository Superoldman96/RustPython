use super::{PyStr, PyStrRef, PyType, PyTypeRef, PyWeak};
use crate::{
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    class::PyClassImpl,
    common::hash::PyHash,
    function::{OptionalArg, PyComparisonValue, PySetterValue},
    protocol::{PyIter, PyIterReturn, PyMappingMethods, PySequenceMethods},
    stdlib::builtins::reversed,
    types::{
        AsMapping, AsSequence, Comparable, Constructor, GetAttr, Hashable, IterNext, Iterable,
        PyComparisonOp, Representable, SetAttr,
    },
};
use std::sync::LazyLock;

#[pyclass(module = false, name = "weakproxy", unhashable = true, traverse)]
#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyRef<PyWeak>,
}

impl PyPayload for PyWeakProxy {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.weakproxy_type
    }
}

#[derive(FromArgs)]
pub struct WeakProxyNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
}

impl Constructor for PyWeakProxy {
    type Args = WeakProxyNewArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { referent, callback }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        // using an internal subclass as the class prevents us from getting the generic weakref,
        // which would mess up the weakref count
        let weak_cls = WEAK_SUBCLASS.get_or_init(|| {
            vm.ctx.new_class(
                None,
                "__weakproxy",
                vm.ctx.types.weakref_type.to_owned(),
                super::PyWeak::make_slots(),
            )
        });
        // TODO: PyWeakProxy should use the same payload as PyWeak
        Self {
            weak: referent.downgrade_with_typ(callback.into_option(), weak_cls.clone(), vm)?,
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

crate::common::static_cell! {
    static WEAK_SUBCLASS: PyTypeRef;
}

#[pyclass(with(
    GetAttr,
    SetAttr,
    Constructor,
    Comparable,
    AsSequence,
    AsMapping,
    Representable,
    IterNext
))]
impl PyWeakProxy {
    fn try_upgrade(&self, vm: &VirtualMachine) -> PyResult {
        self.weak.upgrade().ok_or_else(|| new_reference_error(vm))
    }

    #[pymethod]
    fn __str__(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        self.try_upgrade(vm)?.str(vm)
    }

    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_upgrade(vm)?.length(vm)
    }

    #[pymethod]
    fn __bool__(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_upgrade(vm)?.is_true(vm)
    }

    #[pymethod]
    fn __bytes__(&self, vm: &VirtualMachine) -> PyResult {
        self.try_upgrade(vm)?.bytes(vm)
    }

    #[pymethod]
    fn __reversed__(&self, vm: &VirtualMachine) -> PyResult {
        let obj = self.try_upgrade(vm)?;
        reversed(obj, vm)
    }
    #[pymethod]
    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_upgrade(vm)?.to_sequence().contains(&needle, vm)
    }

    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = self.try_upgrade(vm)?;
        obj.get_item(&*needle, vm)
    }

    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.set_item(&*needle, value, vm)
    }

    fn delitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.del_item(&*needle, vm)
    }
}

impl Iterable for PyWeakProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.try_upgrade(vm)?;
        Ok(obj.get_iter(vm)?.into())
    }
}

impl IterNext for PyWeakProxy {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let obj = zelf.try_upgrade(vm)?;
        PyIter::new(obj).next(vm)
    }
}

fn new_reference_error(vm: &VirtualMachine) -> PyRef<super::PyBaseException> {
    vm.new_exception_msg(
        vm.ctx.exceptions.reference_error.to_owned(),
        "weakly-referenced object no longer exists".to_owned(),
    )
}

impl GetAttr for PyWeakProxy {
    // TODO: callbacks
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.try_upgrade(vm)?;
        obj.get_attr(name, vm)
    }
}

impl SetAttr for PyWeakProxy {
    fn setattro(
        zelf: &Py<Self>,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = zelf.try_upgrade(vm)?;
        obj.call_set_attr(vm, attr_name, value)
    }
}

impl Comparable for PyWeakProxy {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let obj = zelf.try_upgrade(vm)?;
        Ok(PyComparisonValue::Implemented(
            obj.rich_compare_bool(other, op, vm)?,
        ))
    }
}

impl AsSequence for PyWeakProxy {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| PyWeakProxy::sequence_downcast(seq).len(vm)),
            contains: atomic_func!(|seq, needle, vm| {
                PyWeakProxy::sequence_downcast(seq).__contains__(needle.to_owned(), vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsMapping for PyWeakProxy {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, vm| PyWeakProxy::mapping_downcast(mapping).len(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyWeakProxy::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyWeakProxy::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.setitem(needle.to_owned(), value, vm)
                } else {
                    zelf.delitem(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl Representable for PyWeakProxy {
    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        zelf.try_upgrade(vm)?.repr(vm)
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub fn init(context: &Context) {
    PyWeakProxy::extend_class(context, context.types.weakproxy_type);
}

impl Hashable for PyWeakProxy {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.try_upgrade(vm)?.hash(vm)
    }
}
