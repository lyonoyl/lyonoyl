/**
 * The frame a message is drawn in, and the only frame in this application.
 *
 * The third of the four layers between a message and the person reading it: the message is
 * sanitized in Rust, served on a scheme of its own under a policy of its own, and drawn here in a
 * sandbox with nothing allowed in it, sending no referrer. No `allow-*`, ever — one of them turns a
 * frame holding a hostile message into a place that can run it, and the window it sits in is the
 * one that holds IPC. The reader and the composer draw a message through this one component, so
 * these attributes are written once.
 */
export function MessageFrame({
  title,
  url,
  className,
}: {
  /** What the frame is called to somebody whose screen is read aloud. */
  title: string;
  /** The page the core serves for it, or nothing while it is being asked for. */
  url: string | undefined;
  className: string;
}) {
  return (
    <iframe
      title={title}
      src={url}
      sandbox=""
      referrerPolicy="no-referrer"
      className={className}
    />
  );
}
