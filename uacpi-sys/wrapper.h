// Headers that bindgen turns into Rust declarations.
#include <uacpi/acpi.h>
#include <uacpi/context.h>
#include <uacpi/kernel_api.h>
#include <uacpi/log.h>
#include <uacpi/namespace.h>
#include <uacpi/status.h>
#include <uacpi/tables.h>
#include <uacpi/types.h>
#include <uacpi/uacpi.h>
#include <uacpi/utilities.h>

// uACPI keeps this in an internal header, but it is a normal exported function and there is no
// public way to get at the object attached to a namespace node without evaluating it.
uacpi_object *uacpi_namespace_node_get_object(const uacpi_namespace_node *node);
