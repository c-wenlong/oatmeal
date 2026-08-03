import EventKit
import Foundation
import SidecarProtocol

/// Reads the user's calendar through EventKit.
///
/// **EventKit rather than Google/Microsoft OAuth**, which is a deliberate
/// departure from the roadmap. macOS already holds the user's calendars —
/// Google, Exchange, iCloud, whatever they have added — and EventKit reads all
/// of them through one local API. Going direct to each provider would mean
/// registering OAuth clients, shipping a client secret inside a local-first app
/// that promises nothing leaves the machine, storing refresh tokens, and
/// handling two more auth flows. For an app whose entire premise is "your data
/// stays here", reading the calendar the OS already syncs is both less code and
/// a better promise.
///
/// The trade is that a user whose calendar is not in macOS Calendar sees
/// nothing. That is recoverable later by adding a provider path; shipping a
/// secret is not.
final class CalendarWatcher {
    /// How often the calendar is re-read.
    ///
    /// Five minutes, per the roadmap. Events are fetched for a window well
    /// ahead of now, so the poll interval only bounds how quickly a *newly
    /// created* meeting is noticed, not how punctually an existing one fires.
    private let interval: TimeInterval = 300

    /// How far ahead to look.
    private let horizon: TimeInterval = 24 * 3600

    private let store = EKEventStore()
    private let onEvents: ([CalendarEvent]) -> Void
    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "oatmeal.sidecar.calendar")

    init(onEvents: @escaping ([CalendarEvent]) -> Void) {
        self.onEvents = onEvents
    }

    /// Whether the user has granted calendar access.
    static var isAuthorized: Bool {
        let status = EKEventStore.authorizationStatus(for: .event)
        if #available(macOS 14.0, *) {
            return status == .fullAccess
        }
        return status == .authorized
    }

    /// Asks for calendar access. Safe to call when already granted.
    func requestAccess(_ completion: @escaping (Bool) -> Void) {
        store.requestFullAccessToEvents { granted, error in
            if let error {
                Log.info("Calendar access failed: \(error)")
            }
            completion(granted)
        }
    }

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: interval)
        timer.setEventHandler { [weak self] in self?.poll() }
        timer.resume()
        self.timer = timer
        Log.info("Calendar watcher started.")
    }

    func stop() {
        timer?.cancel()
        timer = nil
    }

    private func poll() {
        guard Self.isAuthorized else {
            // Not an error worth shouting about on a loop: the user simply has
            // not granted access, and detection works without it.
            return
        }
        onEvents(fetch())
    }

    /// Reads the upcoming window.
    ///
    /// Every event is reported, meeting-shaped or not. The "is this a meeting"
    /// judgement lives in Rust, where it is pure and tested — this side stays a
    /// dumb reader so the rule can change without touching the sidecar.
    func fetch(now: Date = Date()) -> [CalendarEvent] {
        let predicate = store.predicateForEvents(
            withStart: now, end: now.addingTimeInterval(horizon), calendars: nil)

        return store.events(matching: predicate).compactMap { event in
            guard let start = event.startDate else { return nil }
            // All-day entries are holidays, birthdays and out-of-office blocks.
            // Offering to record one is never right.
            guard !event.isAllDay else { return nil }

            return CalendarEvent(
                id: event.eventIdentifier ?? UUID().uuidString,
                title: event.title,
                startsAt: Int(start.timeIntervalSince1970 * 1000),
                endsAt: event.endDate.map { Int($0.timeIntervalSince1970 * 1000) },
                location: event.location,
                // `url` is where Calendar.app puts a conferencing link; some
                // providers only put it in the notes, so both are sent and the
                // extraction happens on the Rust side.
                url: event.url?.absoluteString,
                notes: event.notes,
                attendeeCount: event.attendees?.count ?? 0)
        }
    }
}
