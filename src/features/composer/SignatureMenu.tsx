import { useRef, useState } from "react";
import { Signature } from "lucide-react";

import { useTranslation } from "@/shared/i18n";
import { Button } from "@/shared/ui/primitives/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/primitives/dropdown-menu";

import { signatureName, type Standing } from "./signature";
import { SignaturePanel } from "./SignaturePanel";

/**
 * The signature where a message is written, as the mail clients people know have it there: the
 * mailbox's signature or none in this message, and the signature itself written or changed.
 *
 * Without it the signature lives only in the settings, and a person writing a message has no way
 * to know there is one to have.
 */
export function SignatureMenu({
  account,
  name,
  signature,
  standing,
  onSign,
  onKept,
  disabled,
}: {
  /** The mailbox the message is written from. */
  account: string;
  /** What the mailbox is called. */
  name: string;
  /** Its signature, empty for none. */
  signature: string;
  /** Where the message's own signature stands. */
  standing: Standing;
  /** Signs the message with this signature, or with none for an empty one. */
  onSign: (signature: string) => void;
  /** Told the signature as the core kept it, so every message signs with it from now on. */
  onKept: (account: string, signature: string) => void;
  disabled: boolean;
}) {
  const t = useTranslation();
  const [editing, setEditing] = useState(false);
  /**
   * Whether writing the signature was chosen, done once the menu has closed: the menu holds the
   * focus until it is gone, and would take it back from the panel's field and drop it on nothing.
   */
  const chosen = useRef(false);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="outline" size="sm" disabled={disabled}>
            <Signature />
            {t("compose.signature")}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          // As wide as its longest line rather than as the button, at whose width a signature's
          // first line has no room.
          className="w-auto max-w-(--radix-dropdown-menu-content-available-width)"
          onCloseAutoFocus={() => {
            if (!chosen.current) return;
            chosen.current = false;
            setEditing(true);
          }}
        >
          {/* Offered while the message's signature stands as it was put, or there is none: one
              somebody changed by hand is theirs, and neither choice could say what it is. */}
          {signature !== "" && standing !== "changed" && (
            <>
              <DropdownMenuRadioGroup
                value={standing}
                onValueChange={(value) =>
                  onSign(value === "signed" ? signature : "")
                }
              >
                <DropdownMenuRadioItem value="signed">
                  <bdi className="max-w-64 truncate">
                    {signatureName(signature)}
                  </bdi>
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="unsigned">
                  {t("compose.noSignature")}
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
              <DropdownMenuSeparator />
            </>
          )}
          <DropdownMenuItem
            onSelect={() => {
              chosen.current = true;
            }}
          >
            {signature === ""
              ? t("compose.addSignature")
              : t("compose.editSignature")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {editing && (
        <SignaturePanel
          account={account}
          name={name}
          signature={signature}
          onKept={(kept) => {
            // Into this message too, where its signature stands or when the mailbox had none to
            // put there: the one written is the one somebody is looking at. Not into a message
            // somebody chose to send without one.
            if (standing === "signed" || signature === "") onSign(kept);
            onKept(account, kept);
          }}
          onClose={() => setEditing(false)}
        />
      )}
    </>
  );
}
