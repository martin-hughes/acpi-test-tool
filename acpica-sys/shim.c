/*
 * ACPICA prints all of its diagnostics through AcpiOsPrintf, which takes C varargs. Rust can't
 * define variadic functions on stable, so the formatting is done here, and the finished message
 * is handed to a plain function that the Rust host layer provides.
 */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

#include "acpi.h"

/* Provided by the Rust host layer. Takes a NUL-terminated message. */
extern void acpica_host_log(const char *message);

void AcpiOsVprintf(const char *Format, va_list Args)
{
    char stack_buffer[512];
    char *message = stack_buffer;
    va_list copy;
    int length;

    va_copy(copy, Args);
    length = vsnprintf(stack_buffer, sizeof(stack_buffer), Format, copy);
    va_end(copy);

    if (length < 0)
        return;

    if ((size_t) length >= sizeof(stack_buffer))
    {
        /* Too long for the stack buffer: format again into one that fits. */
        message = malloc((size_t) length + 1);
        if (!message)
            return;
        vsnprintf(message, (size_t) length + 1, Format, Args);
    }

    acpica_host_log(message);

    if (message != stack_buffer)
        free(message);
}

void AcpiOsPrintf(const char *Format, ...)
{
    va_list args;

    va_start(args, Format);
    AcpiOsVprintf(Format, args);
    va_end(args);
}
