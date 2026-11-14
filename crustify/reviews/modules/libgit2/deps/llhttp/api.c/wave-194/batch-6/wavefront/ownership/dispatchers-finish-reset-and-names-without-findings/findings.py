import json, sys, os
out = sys.argv[1]
def ptr(scalar=None, array=None, string=False, owned=False, lifetime=None, nullable=False, mutable=False, note=""):
    return {"scalar": scalar, "array": array, "string": string, "owned": owned,
            "borrowed": {"lifetime": lifetime} if lifetime else None,
            "nullable": nullable, "mutable": mutable, "note": note}
S = lambda note: ptr(scalar={"by_val": True}, lifetime="arg:s", mutable=True, note=note)
spans = ["url","status","method","version","header_field","header_value","body","chunk_extension_name","chunk_extension_value"]
events = ["message_begin","url_complete","status_complete","method_complete","version_complete","header_field_complete","header_value_complete","headers_complete","message_complete","chunk_header","chunk_extension_name_complete","chunk_extension_value_complete","chunk_complete","reset"]
docs = {}
for n in spans:
    docs["llhttp__on_" + n] = {"ptr_args": {
        "s": {"ptr": S(f"The parser llhttp's generated parser is executing, whose table's `on_{n}` is called with it for the call, and whose `reason` is set on a `-1`; not kept. Ported as ParserMut::on_{n}(&[u8]).")},
        "p": {"ptr": ptr(array={"by_val": True}, lifetime="arg:p", nullable=True, note="The start of the run, within the input of the llhttp_execute call that runs the parse: where the span starts, or the input's start. Read for the call alone, `endp - p` bytes, and handed to the slot's callback; not kept. NULL only for an empty run (endp == p) of a NULL input.")},
        "endp": {"ptr": ptr(array={"by_val": True}, lifetime="arg:endp", nullable=True, note="One past the run's last byte, in the same input as `p` and at or after it; only subtracted from, never read. The run's length is `endp - p`.")},
    }, "lifetime": None}
for n in events:
    docs["llhttp__on_" + n] = {"ptr_args": {
        "s": {"ptr": S(f"The parser llhttp's generated parser is executing, whose table's `on_{n}` is called with it for the call; not kept. Ported as ParserMut::on_{n}().")},
        "p": {"ptr": ptr(array={"by_val": True}, lifetime="arg:p", nullable=True, note="Where the parse is in its input; never read, the event having no run. The export ignores it.")},
        "endp": {"ptr": ptr(array={"by_val": True}, lifetime="arg:endp", nullable=True, note="The end of the parse's input; never read. The export ignores it.")},
    }, "lifetime": None}
docs["llhttp_finish"] = {"ptr_args": {"parser": {"ptr": ptr(scalar={"by_val": True}, lifetime="arg:parser", mutable=True, note="The parser told its input has ended: its `finish` state is read, its table's `on_message_complete` called with it on HTTP_FINISH_SAFE_WITH_CB, and its `reason` set on HTTP_FINISH_UNSAFE; not kept. Its error is left as it was. Ported as ParserMut::finish().")}}, "lifetime": None}
docs["llhttp_reset"] = {"ptr_args": {"parser": {"ptr": ptr(scalar={"by_val": True}, lifetime="arg:parser", mutable=True, note="An initialised parser, re-initialised in place by llhttp__internal_init, keeping its type, settings, data and lenient flags, which it reads first: so no initialiser of raw storage, unlike llhttp_init. Not kept. Ported as ParserMut::reset().")}}, "lifetime": None}
for n, what in [("llhttp_errno_name", "\"HPE_\" #NAME, the errno's constant name; Errno::constant_name"), ("llhttp_method_name", "#STRING of HTTP_ALL_METHOD_MAP's entry; Method::map_string"), ("llhttp_status_name", "#STRING of HTTP_STATUS_MAP's entry; Status::map_string")]:
    docs[n] = {"ptr_args": {}, "ptr_ret": {"ptr": ptr(string=True, lifetime="static", note=f"A string literal of api.c, {what}. Never NULL: a value no entry names aborts. Not owned; never freed.")}, "lifetime": None}
for n, d in docs.items():
    json.dump(d, open(os.path.join(out, n + ".json"), "w"), indent=1)
print(len(docs))
