import { forwardRef, type SVGProps } from "react";

/// Lightspeed bot mark: a round little bot leaning into its own speed,
/// motion streaks trailing behind. The whole glyph paints with
/// `currentColor` and the eyes are punched out of the body, so one drawing
/// serves every context: in the sidebar it inherits the text color and
/// reads monochrome; on cards and avatars set `color` per bot.
type BotMarkProps = SVGProps<SVGSVGElement> & { size?: number | string };

export const BotMark = forwardRef<SVGSVGElement, BotMarkProps>(
  ({ size = 24, ...props }, ref) => (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      aria-hidden="true"
      {...props}
    >
      <g stroke="currentColor" strokeWidth={3.5} strokeLinecap="round">
        <path d="M6 15h6.5" />
        <path d="M2 25h7" />
        <path d="M6.5 35h6" />
      </g>
      <path
        transform="rotate(7 28 25)"
        fill="currentColor"
        fillRule="evenodd"
        d="M12 25a16 16 0 1 0 32 0 16 16 0 1 0-32 0ZM21.4 19.2a2.2 2.2 0 0 1 4.4 0v4.6a2.2 2.2 0 0 1-4.4 0ZM30.2 19.2a2.2 2.2 0 0 1 4.4 0v4.6a2.2 2.2 0 0 1-4.4 0Z"
      />
    </svg>
  ),
);
BotMark.displayName = "BotMark";

/// The bot without his speed streaks — same lean, same punched-out eyes,
/// centered in the viewBox. For tight spots where the trailing lines crowd
/// (favicon, tiny avatars, badges).
export const BotFace = forwardRef<SVGSVGElement, BotMarkProps>(
  ({ size = 24, ...props }, ref) => (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      aria-hidden="true"
      {...props}
    >
      <path
        transform="rotate(7 24 24)"
        fill="currentColor"
        fillRule="evenodd"
        d="M8 24a16 16 0 1 0 32 0 16 16 0 1 0-32 0ZM17.4 18.2a2.2 2.2 0 0 1 4.4 0v4.6a2.2 2.2 0 0 1-4.4 0ZM26.2 18.2a2.2 2.2 0 0 1 4.4 0v4.6a2.2 2.2 0 0 1-4.4 0Z"
      />
    </svg>
  ),
);
BotFace.displayName = "BotFace";
