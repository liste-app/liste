// A global hotkey through Carbon's RegisterEventHotKey, which works from
// any app state without accessibility permissions. Option-Space by default.

import AppKit
import Carbon.HIToolbox

@MainActor
final class HotKey {
    static let space = UInt32(kVK_Space)
    static let option = UInt32(optionKey)

    private var ref: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private let id: UInt32
    private static var next: UInt32 = 1
    private static var handlers: [UInt32: () -> Void] = [:]

    init(keyCode: UInt32, modifiers: UInt32, handler: @escaping @MainActor () -> Void) {
        id = HotKey.next
        HotKey.next += 1
        HotKey.handlers[id] = handler
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(
            GetApplicationEventTarget(),
            { _, event, _ -> OSStatus in
                var hotKeyID = EventHotKeyID()
                GetEventParameter(
                    event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                    nil, MemoryLayout<EventHotKeyID>.size, nil, &hotKeyID)
                let id = hotKeyID.id
                MainActor.assumeIsolated {
                    HotKey.handlers[id]?()
                }
                return noErr
            },
            1, &spec, nil, &handlerRef)
        let hotKeyID = EventHotKeyID(signature: OSType(0x4C53_5445), id: id)  // "LSTE"
        let status = RegisterEventHotKey(keyCode, modifiers, hotKeyID, GetApplicationEventTarget(), 0, &ref)
        if status != noErr {
            log.error("hotkey registration failed: \(status)")
        }
    }

    /// Release the hotkey. Called explicitly, since Carbon handles are not
    /// safe to touch from a nonisolated deinit.
    func unregister() {
        if let ref { UnregisterEventHotKey(ref) }
        if let handlerRef { RemoveEventHandler(handlerRef) }
        ref = nil
        handlerRef = nil
        HotKey.handlers[id] = nil
    }
}
