# Errors shown to the user.

# $file: the file's name or path, as the user gave it.
error-file-not-found = There's no file called { $file }.
# Shown as the reason a file couldn't be opened: it was being written while Legible read it.
error-file-changed-while-read = the file changed while it was being read
# Shown as the reason a save failed: Legible writes beside the file first, and found no free name.
error-no-free-name = every name beside the file is taken

## Why something failed: the { $reason } of the messages that say so (error-open,
## chrome-save-file-failed and the rest). Each follows a colon or sits in brackets, so it starts
## with a small letter and has no full stop. Other failures are said in Windows' own words.

# The file or folder isn't there (any more).
reason-not-found = it isn't there
# Windows refused: the reader may not open or change it.
reason-denied = Windows didn't allow it
# Another program has the file open, and won't share it.
reason-in-use = another program is using it
# The disk, or the file, can't be written to.
reason-read-only = it's read-only
reason-disk-full = the disk is full
# A failure of Legible's own, which only its log explains.
reason-unexpected = something unexpected went wrong
